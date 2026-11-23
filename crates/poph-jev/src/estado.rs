//! O que o Jev enxerga: a conversa recente, a política da empresa e o que o agente pode fazer.
//!
//! Regras que valem para todo gate:
//! - **Só o necessário sai do Harness.** Mensagem truncada em [`MAX_CHARS_MENSAGEM`] e no máximo
//!   [`MAX_MENSAGENS`] mensagens — a janela do POP Atende (`_HISTORY_MAX = 10`).
//! - **Nada de dado pessoal extra.** Vai o texto como está; telefone, nome e id não são acrescentados
//!   aqui (o texto do cliente já é o dado, e é ele que a decisão precisa).
//! - **O estado é a chave do cache.** Mesmo estado ⇒ mesma pergunta ⇒ mesma decisão, sem segunda
//!   chamada.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Quantas mensagens da conversa vão para o Jev (janela do POP Atende).
pub const MAX_MENSAGENS: usize = 10;
/// Teto de caracteres por mensagem (o POP Atende trunca em 800).
pub const MAX_CHARS_MENSAGEM: usize = 800;
/// Quantas situações de transferência da empresa vão (o POP Atende usa 15).
pub const MAX_SITUACOES: usize = 15;
/// Teto de caracteres por situação.
pub const MAX_CHARS_SITUACAO: usize = 300;
/// Quantos produtos candidatos do G5 vão no estado (o top-k do RAG é 3 por padrão).
pub const MAX_CANDIDATOS_PRODUTO: usize = 10;
/// Teto de caracteres por campo do candidato (nome, detalhe, id).
pub const MAX_CHARS_CANDIDATO: usize = 300;
/// Teto de caracteres da resposta em revisão (G6). O envio do WhatsApp corta em 4000, então o
/// texto que vai para o Jev cabe inteiro nos casos reais.
pub const MAX_CHARS_RESPOSTA: usize = 4000;
/// Teto de caracteres do texto extraído de arquivo enviado ao Jev (G7). É o mesmo teto que a
/// extração aplica por arquivo (HR-23); juntar todos os anexos de um turno pode passar disso.
pub const MAX_CHARS_ARQUIVO: usize = 20_000;
/// Quantos fluxos liberados podem virar opção do G9.
///
/// É o mesmo teto que a API impõe ao liberar fluxo para um agente (FX-08): acima disso o gate não
/// se aplica e avisa, porque a pergunta ao Jev precisa de opções finitas.
pub const MAX_FLUXOS_LIBERADOS: usize = 20;
/// Teto de caracteres de cada pedaço do gatilho de um fluxo (`quando`, `nao_quando`, cada exemplo).
///
/// O gatilho é escrito pelo autor do fluxo e vira o critério de uma opção do G9; sem teto, um
/// texto longo entraria inteiro no pedido ao Jev, em todas as perguntas do lote.
pub const MAX_CHARS_GATILHO: usize = 400;
/// Quantos exemplos de fala do cliente por fluxo vão no estado (o gatilho do PRD traz dois).
pub const MAX_EXEMPLOS_FLUXO: usize = 5;

/// Quem falou.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PapelJev {
    /// Cliente.
    Cliente,
    /// Agente.
    Agente,
}

impl PapelJev {
    /// Texto do papel no estado enviado ao Jev.
    pub fn como_texto(self) -> &'static str {
        match self {
            PapelJev::Cliente => "cliente",
            PapelJev::Agente => "agente",
        }
    }
}

/// Um produto candidato do catálogo (o que o G5 escolhe).
///
/// Espelha o produto que a busca de produtos do turno devolveu, sem os campos que o Jev não julga
/// (id externo, notas da busca). O `id` é a opção enviada ao Jev e a chave que o **código** valida
/// de volta: o modelo nunca escolhe fora desta lista (HR-33).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidatoProduto {
    /// Id do produto (a opção enviada ao Jev).
    pub id: String,
    /// Nome do produto.
    pub nome: String,
    /// O que ajuda o Jev a reconhecer o produto: categoria, preço e unidade, quando houver.
    #[serde(default)]
    pub detalhe: String,
}

/// Um fluxo liberado ao agente, como o G9 o apresenta ao Jev.
///
/// Espelha o `gatilho` da revisão publicada do fluxo (`quando`, `nao_quando`, `exemplos`), sem os
/// nós nem as regras: o Jev só decide **se** e **qual** fluxo iniciar; quem valida e inicia é o
/// código. O `id` é a opção enviada e a chave que o código confere de volta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FluxoJev {
    /// Id do fluxo (a opção enviada ao Jev).
    pub id: String,
    /// Nome do fluxo, para o modelo saber o que ele faz quando o gatilho é curto.
    pub nome: String,
    /// Quando o fluxo se aplica, nas palavras do autor do fluxo.
    #[serde(default)]
    pub quando: String,
    /// Quando **não** se aplica (o cliente está só perguntando, por exemplo).
    #[serde(default)]
    pub nao_quando: String,
    /// Exemplos de fala de cliente que pedem este fluxo.
    #[serde(default)]
    pub exemplos: Vec<String>,
}

/// Uma mensagem da conversa, como o Jev a lê.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MensagemJev {
    /// Quem falou.
    pub papel: PapelJev,
    /// Texto (já truncado).
    pub texto: String,
}

impl MensagemJev {
    /// Mensagem do cliente (truncada).
    pub fn cliente(texto: impl Into<String>) -> Self {
        Self::nova(PapelJev::Cliente, texto)
    }

    /// Mensagem do agente (truncada).
    pub fn agente(texto: impl Into<String>) -> Self {
        Self::nova(PapelJev::Agente, texto)
    }

    fn nova(papel: PapelJev, texto: impl Into<String>) -> Self {
        Self {
            papel,
            texto: truncar(&texto.into(), MAX_CHARS_MENSAGEM),
        }
    }
}

/// Estado enviado ao Jev: é dele que cada gate monta a pergunta e é ele que identifica a decisão no
/// cache.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EstadoJev {
    /// Canal da conversa (`whatsapp`, `console`, `cli`).
    #[serde(default)]
    pub canal: String,
    /// Mensagens recentes, da mais antiga para a mais nova.
    #[serde(default)]
    pub mensagens: Vec<MensagemJev>,
    /// Situações em que a empresa manda transferir (política do cliente).
    #[serde(default)]
    pub situacoes_de_handoff: Vec<String>,
    /// Departamentos ativos da org (candidatos do G4).
    #[serde(default)]
    pub departamentos: Vec<String>,
    /// O agente aceita transferência para humano.
    #[serde(default)]
    pub transferencia_habilitada: bool,
    /// O agente tem base de conhecimento com conteúdo.
    #[serde(default)]
    pub base_disponivel: bool,
    /// O agente tem catálogo de produtos ativo.
    #[serde(default)]
    pub produtos_disponiveis: bool,
    /// Candidatos da busca de produtos deste turno (opções do G5). Vazio = o G5 não se aplica.
    #[serde(default)]
    pub produtos_candidatos: Vec<CandidatoProduto>,
    /// Resposta que o agente está prestes a enviar, para a revisão do G6. `None` = não há o que
    /// revisar (o G6 não pergunta).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resposta_candidata: Option<String>,
    /// Fluxos liberados ao agente (opções do G9). Vazio = o G9 não pergunta.
    ///
    /// Cada item traz o gatilho do fluxo, que é o que o Jev lê para escolher. A lista **não** é
    /// cortada em [`MAX_FLUXOS_LIBERADOS`]: o gate precisa ver o total para se declarar inaplicável
    /// acima do teto em vez de perguntar com opções faltando. Quem monta o estado é o turno, a
    /// partir do que o `poph-store` devolve (a API já recusa liberar o 21º fluxo).
    #[serde(default)]
    pub fluxos_liberados: Vec<FluxoJev>,
    /// Texto extraído dos arquivos que o cliente enviou neste turno (G7), já entre as marcas de
    /// conteúdo não confiável. `None` = o turno não tem texto de arquivo para julgar.
    ///
    /// Só a chamada do G7 precisa dele, mas ele acompanha o estado porque a injeção pode estar
    /// **dentro** do documento: o texto do cliente e o texto do arquivo são julgados juntos.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texto_de_arquivo: Option<String>,
}

impl EstadoJev {
    /// Estado de uma conversa no canal dado.
    pub fn novo(canal: impl Into<String>) -> Self {
        Self {
            canal: canal.into(),
            ..Self::default()
        }
    }

    /// Substitui as mensagens: guarda as [`MAX_MENSAGENS`] últimas e trunca cada uma.
    pub fn com_mensagens(mut self, mensagens: impl IntoIterator<Item = MensagemJev>) -> Self {
        let mut todas: Vec<MensagemJev> = mensagens.into_iter().collect();
        if todas.len() > MAX_MENSAGENS {
            todas.drain(..todas.len() - MAX_MENSAGENS);
        }
        self.mensagens = todas;
        self
    }

    /// Define a política de transferência (situações e se o agente transfere).
    pub fn com_handoff(mut self, situacoes: Vec<String>, habilitada: bool) -> Self {
        self.situacoes_de_handoff = situacoes
            .into_iter()
            .take(MAX_SITUACOES)
            .map(|situacao| truncar(&situacao, MAX_CHARS_SITUACAO))
            .collect();
        self.transferencia_habilitada = habilitada;
        self
    }

    /// Define os departamentos candidatos do G4.
    pub fn com_departamentos(mut self, departamentos: Vec<String>) -> Self {
        self.departamentos = departamentos;
        self
    }

    /// Define o que o agente tem à mão (base e produtos).
    pub fn com_capacidades(mut self, base_disponivel: bool, produtos_disponiveis: bool) -> Self {
        self.base_disponivel = base_disponivel;
        self.produtos_disponiveis = produtos_disponiveis;
        self
    }

    /// Define os candidatos do G5 (os produtos que a busca do turno devolveu).
    ///
    /// Limita a [`MAX_CANDIDATOS_PRODUTO`] e trunca nome e detalhe: o candidato vem do cadastro e
    /// não pode inflar o pedido ao Jev.
    pub fn com_produtos_candidatos(mut self, candidatos: Vec<CandidatoProduto>) -> Self {
        self.produtos_candidatos = candidatos
            .into_iter()
            .take(MAX_CANDIDATOS_PRODUTO)
            .map(|candidato| CandidatoProduto {
                id: truncar(&candidato.id, MAX_CHARS_CANDIDATO),
                nome: truncar(&candidato.nome, MAX_CHARS_CANDIDATO),
                detalhe: truncar(&candidato.detalhe, MAX_CHARS_CANDIDATO),
            })
            .collect();
        self
    }

    /// Define a resposta em revisão (G6): o texto que o agente vai enviar ao cliente.
    ///
    /// Trunca em [`MAX_CHARS_RESPOSTA`] para o pedido ao Jev não crescer com a resposta.
    pub fn com_resposta_candidata(mut self, texto: impl Into<String>) -> Self {
        self.resposta_candidata = Some(truncar(&texto.into(), MAX_CHARS_RESPOSTA));
        self
    }

    /// Define o texto extraído dos arquivos do turno (G7).
    ///
    /// O texto chega já envelopado pelas marcas de conteúdo não confiável (`poph-files`), e é
    /// truncado em [`MAX_CHARS_ARQUIVO`] para o pedido não crescer com um documento enorme.
    pub fn com_texto_de_arquivo(mut self, texto: impl Into<String>) -> Self {
        self.texto_de_arquivo = Some(truncar(&texto.into(), MAX_CHARS_ARQUIVO));
        self
    }

    /// Define os fluxos liberados ao agente (opções do G9).
    ///
    /// Corta cada pedaço do gatilho em [`MAX_CHARS_GATILHO`] e guarda no máximo
    /// [`MAX_EXEMPLOS_FLUXO`] exemplos, para o pedido ao Jev não crescer com texto de autor de
    /// fluxo. O corte fica aqui, num lugar só — como os outros tetos do estado —, e o critério do
    /// gate compõe o que já veio cortado.
    pub fn com_fluxos_liberados(mut self, fluxos: impl IntoIterator<Item = FluxoJev>) -> Self {
        self.fluxos_liberados = fluxos
            .into_iter()
            .map(|fluxo| FluxoJev {
                id: truncar(&fluxo.id, MAX_CHARS_CANDIDATO),
                nome: truncar(&fluxo.nome, MAX_CHARS_CANDIDATO),
                quando: truncar(&fluxo.quando, MAX_CHARS_GATILHO),
                nao_quando: truncar(&fluxo.nao_quando, MAX_CHARS_GATILHO),
                exemplos: fluxo
                    .exemplos
                    .into_iter()
                    .take(MAX_EXEMPLOS_FLUXO)
                    .map(|exemplo| truncar(&exemplo, MAX_CHARS_GATILHO))
                    .collect(),
            })
            .collect();
        self
    }

    /// Última mensagem da conversa (o que os gates julgam).
    pub fn ultima_mensagem(&self) -> Option<&MensagemJev> {
        self.mensagens.last()
    }
    /// Última mensagem do cliente (o POP Atende julga a última fala do cliente).
    pub fn ultima_do_cliente(&self) -> Option<&MensagemJev> {
        self.mensagens
            .iter()
            .rev()
            .find(|mensagem| mensagem.papel == PapelJev::Cliente)
    }

    /// O estado como vai no campo `state` do pedido.
    ///
    /// O desenho é o do POP Atende: a conversa, a política da empresa e o que o agente pode fazer.
    pub fn para_json(&self) -> Value {
        let mut valor = json!({
            "canal": self.canal,
            "conversa": {
                "mensagens": self
                    .mensagens
                    .iter()
                    .map(|mensagem| json!({
                        "papel": mensagem.papel.como_texto(),
                        "texto": mensagem.texto,
                    }))
                    .collect::<Vec<_>>(),
            },
            "politica": {
                "situacoes_de_handoff": self.situacoes_de_handoff,
                "transferencia_habilitada": self.transferencia_habilitada,
                "departamentos": self.departamentos,
            },
            "capacidades": {
                "base_de_conhecimento": self.base_disponivel,
                "produtos": self.produtos_disponiveis,
            },
            // Candidatos do G5 (HR-33): as opções que o Jev pode escolher. Vazio nos turnos em que
            // o gate não se aplica — o G5 então não pergunta.
            "catalogo": {
                "candidatos": self
                    .produtos_candidatos
                    .iter()
                    .map(|candidato| json!({
                        "id": candidato.id,
                        "nome": candidato.nome,
                        "detalhe": candidato.detalhe,
                    }))
                    .collect::<Vec<_>>(),
            },
        });
        // Resposta em revisão do G6 (HR-34): só aparece na chamada de revisão, para o estado dos
        // outros gates (e dos conjuntos já medidos) não mudar de forma.
        if let Some(resposta) = &self.resposta_candidata {
            valor["resposta_candidata"] = json!(resposta);
        }
        // Fluxos liberados (G9): só aparecem quando existem, para o estado dos outros gates (e os
        // conjuntos já medidos) não mudar de forma.
        if !self.fluxos_liberados.is_empty() {
            valor["fluxos"] = json!({
                "liberados": self
                    .fluxos_liberados
                    .iter()
                    .map(|fluxo| json!({
                        "id": fluxo.id,
                        "nome": fluxo.nome,
                        "quando": fluxo.quando,
                        "nao_quando": fluxo.nao_quando,
                        "exemplos": fluxo.exemplos,
                    }))
                    .collect::<Vec<_>>(),
            });
        }
        // Texto de arquivo do turno (G7): só aparece quando existe, para o estado dos outros gates
        // (e os conjuntos já medidos) não mudar de forma.
        if let Some(texto) = &self.texto_de_arquivo {
            valor["arquivo"] = json!({ "texto": texto });
        }
        valor
    }

    /// Identificador do estado, usado como chave do cache de decisão.
    ///
    /// FNV-1a de 64 bits sobre o JSON canônico: estável dentro do processo e entre execuções, sem
    /// dependência nova. O cache é por processo, então não precisa de hash criptográfico.
    pub fn hash(&self) -> u64 {
        let texto = serde_json::to_string(&self.para_json()).unwrap_or_default();
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in texto.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }
}

/// Corta o texto em `max` caracteres (na fronteira de caractere, não de byte).
fn truncar(texto: &str, max: usize) -> String {
    if texto.chars().count() <= max {
        return texto.to_owned();
    }
    let mut cortado: String = texto.chars().take(max).collect();
    cortado.push('…');
    cortado
}
