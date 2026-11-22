//! Leitor de linhas com teto de tamanho.
//!
//! O plugin fala uma mensagem JSON por linha. Um plugin mal comportado pode mandar uma linha de
//! gigabytes (ou nenhuma linha e gigabytes de bytes), e o supervisor não pode crescer junto: o
//! buffer é limitado, a linha acima do teto é **descartada até o próximo `\n`** e o supervisor
//! conta o descarte como lixo — em vez de estourar a memória do processo.

use std::io;

use tokio::io::{AsyncRead, AsyncReadExt};

/// Resultado de uma leitura.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Proxima {
    /// Linha completa, sem `\n` (e sem `\r` do CRLF).
    Linha(String),
    /// Linha acima do teto: foi jogada fora. O chamador deve contar como lixo.
    Descartada,
    /// O outro lado fechou a saída.
    Fim,
}

/// Leitor incremental, com teto por linha.
pub struct LeitorLinhas<R> {
    leitura: R,
    buffer: Vec<u8>,
    pedaco: Box<[u8; 8192]>,
    limite: usize,
    descartando: bool,
}

impl<R: AsyncRead + Unpin> LeitorLinhas<R> {
    /// Cria o leitor com o teto de bytes por linha.
    pub fn novo(leitura: R, limite: usize) -> Self {
        Self {
            leitura,
            buffer: Vec::new(),
            pedaco: Box::new([0u8; 8192]),
            limite: limite.max(1),
            descartando: false,
        }
    }

    /// Próxima linha (ou descarte, ou fim).
    pub async fn proxima(&mut self) -> io::Result<Proxima> {
        loop {
            if self.descartando {
                if let Some(pos) = self.buffer.iter().position(|b| *b == b'\n') {
                    self.buffer.drain(..=pos);
                    self.descartando = false;
                    return Ok(Proxima::Descartada);
                }
                self.buffer.clear();
            } else if let Some(pos) = self.buffer.iter().position(|b| *b == b'\n') {
                if pos > self.limite {
                    // A linha completa já passa do teto: descarta inteira, sem montar a `String`.
                    self.buffer.drain(..=pos);
                    return Ok(Proxima::Descartada);
                }
                let mut linha: Vec<u8> = self.buffer.drain(..=pos).collect();
                linha.pop();
                if linha.last() == Some(&b'\r') {
                    linha.pop();
                }
                return Ok(Proxima::Linha(String::from_utf8_lossy(&linha).into_owned()));
            } else if self.buffer.len() > self.limite {
                // Passou do teto sem fim de linha: joga tudo fora e ignora até o próximo `\n`.
                self.buffer.clear();
                self.descartando = true;
                continue;
            }

            let lidos = self.leitura.read(&mut self.pedaco[..]).await?;
            if lidos == 0 {
                // Fim de arquivo no meio de uma linha grande: conta o descarte antes de terminar.
                if self.descartando {
                    self.descartando = false;
                    return Ok(Proxima::Descartada);
                }
                return Ok(Proxima::Fim);
            }
            self.buffer.extend_from_slice(&self.pedaco[..lidos]);
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[tokio::test]
    async fn le_linhas_com_crlf_e_vazio() {
        let dados = b"{\"a\":1}\r\n{\"b\":2}\n\n";
        let mut leitor = LeitorLinhas::novo(&dados[..], 1024);
        assert_eq!(
            leitor.proxima().await.unwrap(),
            Proxima::Linha("{\"a\":1}".into())
        );
        assert_eq!(
            leitor.proxima().await.unwrap(),
            Proxima::Linha("{\"b\":2}".into())
        );
        assert_eq!(
            leitor.proxima().await.unwrap(),
            Proxima::Linha(String::new())
        );
        assert_eq!(leitor.proxima().await.unwrap(), Proxima::Fim);
    }

    #[tokio::test]
    async fn linha_gigante_e_descartada_e_a_proxima_volta() {
        let mut dados = Vec::new();
        dados.extend(std::iter::repeat_n(b'x', 4096));
        dados.push(b'\n');
        dados.extend_from_slice(b"{\"ok\":true}\n");
        let mut leitor = LeitorLinhas::novo(&dados[..], 256);
        assert_eq!(leitor.proxima().await.unwrap(), Proxima::Descartada);
        assert_eq!(
            leitor.proxima().await.unwrap(),
            Proxima::Linha("{\"ok\":true}".into())
        );
        assert_eq!(leitor.proxima().await.unwrap(), Proxima::Fim);
    }

    #[tokio::test]
    async fn linha_sem_fim_de_arquivo_aberta_acima_do_teto_e_descartada_no_fim() {
        let dados: Vec<u8> = std::iter::repeat_n(b'y', 1000).collect();
        let mut leitor = LeitorLinhas::novo(&dados[..], 128);
        // Sem `\n`, o descarte só é confirmado quando o outro lado fecha (ou quando vem um `\n`).
        assert_eq!(leitor.proxima().await.unwrap(), Proxima::Descartada);
        assert_eq!(leitor.proxima().await.unwrap(), Proxima::Fim);
    }
}
