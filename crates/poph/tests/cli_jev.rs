//! Testes de ponta a ponta de `poph jev eval` (HR-28).
//!
//! Processo de verdade contra um Jev falso local (porta efêmera): sem rede externa e sem chave real.
//! O que se confere aqui é a linha de comando — códigos de saída, relatório impresso, JSON estável e
//! a recusa quando falta a chave.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CHAVE: &str = "chave-de-teste-nao-vaza";

/// Sobe um Jev falso que sempre responde a escolha e a confiança dadas, com `usage` opcional.
fn servidor_jev(escolha: &'static str, confianca: f64, uso: Option<(u64, u64)>) -> u16 {
    let lista = TcpListener::bind("127.0.0.1:0").expect("porta efêmera");
    let porta = lista.local_addr().expect("endereço").port();
    std::thread::spawn(move || {
        for fluxo in lista.incoming().flatten() {
            let _ = atender(&fluxo, escolha, confianca, uso);
        }
    });
    porta
}

fn atender(
    fluxo: &TcpStream,
    escolha: &str,
    confianca: f64,
    uso: Option<(u64, u64)>,
) -> std::io::Result<()> {
    let mut leitor = BufReader::new(fluxo);
    let mut cabecalhos = String::new();
    loop {
        let mut linha = String::new();
        if leitor.read_line(&mut linha)? == 0 {
            return Ok(());
        }
        if linha == "\r\n" || linha == "\n" {
            break;
        }
        cabecalhos.push_str(&linha);
    }
    let tamanho = cabecalhos
        .lines()
        .find_map(|linha| {
            linha
                .to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|valor| valor.trim().parse::<usize>().unwrap_or(0))
        })
        .unwrap_or(0);
    let mut corpo = vec![0_u8; tamanho];
    leitor.read_exact(&mut corpo)?;

    let mut corpo = serde_json::json!({
        "answers": {
            "g1_transferir": {
                "type": "choice",
                "choice": escolha,
                "probabilities": {escolha: confianca},
                "confidence": confianca
            }
        }
    });
    if let Some((entrada, saida)) = uso {
        corpo["usage"] = serde_json::json!({
            "input_tokens": entrada,
            "output_tokens": saida,
            "total_tokens": entrada + saida
        });
    }
    let json = corpo.to_string();
    let resposta = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
        json.len()
    );
    let mut fluxo = fluxo;
    fluxo.write_all(resposta.as_bytes())?;
    fluxo.flush()
}

struct Cenario {
    pasta: PathBuf,
    porta: u16,
}

impl Cenario {
    fn novo(rotulo: &str, escolha: &'static str) -> Self {
        Self::detalhado(rotulo, escolha, 0.93, Some((30, 4)))
    }

    /// Cenário com a confiança da resposta e o `usage` do lote escolhidos pelo teste.
    fn detalhado(
        rotulo: &str,
        escolha: &'static str,
        confianca: f64,
        uso: Option<(u64, u64)>,
    ) -> Self {
        let pasta =
            std::env::temp_dir().join(format!("poph-cli-jev-{rotulo}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&pasta);
        fs::create_dir_all(pasta.join("dados")).expect("pasta de dados");
        let porta = servidor_jev(escolha, confianca, uso);
        let config = format!(
            "[dados]\ndiretorio = \"{}\"\n\n[jev]\nativo = true\nurl_base = \"http://127.0.0.1:{porta}\"\n",
            pasta.join("dados").display()
        );
        fs::write(pasta.join("poph.toml"), config).expect("configuração");
        Self { pasta, porta }
    }

    fn escrever_conjunto(&self, conteudo: &str) -> PathBuf {
        let caminho = self.pasta.join("g1.jsonl");
        fs::write(&caminho, conteudo).expect("conjunto");
        caminho
    }

    fn poph(&self, args: &[&str], com_chave: bool) -> Output {
        let mut comando = Command::new(env!("CARGO_BIN_EXE_poph"));
        comando
            .args(args)
            .current_dir(&self.pasta)
            .env_clear()
            .env("POPH_CONFIG", self.pasta.join("poph.toml"));
        if com_chave {
            comando.env("TYPESAFE_API_KEY", CHAVE);
        }
        comando.output().expect("executa o poph")
    }

    fn url_do_jev(&self) -> String {
        format!("http://127.0.0.1:{}", self.porta)
    }
}

impl Drop for Cenario {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pasta);
    }
}

fn codigo(saida: &Output) -> i32 {
    saida.status.code().unwrap_or(-1)
}

fn texto(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

const CONJUNTO: &str = "\
{\"estado\": {\"canal\": \"cli\", \"mensagens\": [{\"papel\": \"cliente\", \"texto\": \"quero falar com um atendente\"}], \"transferencia_habilitada\": true}, \"esperado\": \"transferir_agora\"}
{\"estado\": {\"canal\": \"cli\", \"mensagens\": [{\"papel\": \"cliente\", \"texto\": \"qual o prazo?\"}], \"transferencia_habilitada\": true}, \"esperado\": \"continuar_agente\"}
";

#[test]
fn mede_o_conjunto_e_imprime_acerto_abstencao_e_latencia() {
    let cenario = Cenario::novo("mede", "transferir_agora");
    let arquivo = cenario.escrever_conjunto(CONJUNTO);

    let saida = cenario.poph(
        &["jev", "eval", "g1", arquivo.to_str().expect("caminho")],
        true,
    );

    assert_eq!(codigo(&saida), 0, "stderr: {}", texto(&saida.stderr));
    let impresso = texto(&saida.stdout);
    assert!(impresso.contains("Jev · g1"), "saída: {impresso}");
    assert!(impresso.contains("casos: 2"), "saída: {impresso}");
    assert!(impresso.contains("acerto: 1/2"), "saída: {impresso}");
    assert!(impresso.contains("erros: 1"), "saída: {impresso}");
    assert!(impresso.contains("latência:"), "saída: {impresso}");
    assert!(!impresso.contains(CHAVE), "a chave não pode aparecer");
    let _ = cenario.url_do_jev();
}

#[test]
fn relatorio_em_json_e_estavel() {
    let cenario = Cenario::novo("json", "transferir_agora");
    let arquivo = cenario.escrever_conjunto(CONJUNTO);

    let saida = cenario.poph(
        &[
            "jev",
            "eval",
            "g1",
            arquivo.to_str().expect("caminho"),
            "--json",
        ],
        true,
    );

    assert_eq!(codigo(&saida), 0, "stderr: {}", texto(&saida.stderr));
    let valor: serde_json::Value =
        serde_json::from_str(&texto(&saida.stdout)).expect("saída é JSON");
    assert_eq!(valor["versao_relatorio"], 1);
    assert_eq!(valor["relatorio"]["gate"], "g1");
    assert_eq!(valor["relatorio"]["casos"], 2);
    assert_eq!(valor["relatorio"]["acertos"], 1);
    assert_eq!(valor["relatorio"]["erros"], 1);
    assert_eq!(valor["relatorio"]["abstencoes"], 0);
    assert!(
        valor["relatorio"]["detalhes"]
            .as_array()
            .is_some_and(|d| d.len() == 2)
    );
    assert!(!texto(&saida.stdout).contains(CHAVE));
}

#[test]
fn sem_chave_recusa_e_diz_o_que_falta() {
    let cenario = Cenario::novo("sem-chave", "transferir_agora");
    let arquivo = cenario.escrever_conjunto(CONJUNTO);

    let saida = cenario.poph(
        &["jev", "eval", "g1", arquivo.to_str().expect("caminho")],
        false,
    );

    assert_eq!(codigo(&saida), 1);
    assert!(
        texto(&saida.stderr).contains("TYPESAFE_API_KEY"),
        "stderr: {}",
        texto(&saida.stderr)
    );
}

#[test]
fn gate_desconhecido_e_arquivo_ausente_falham_com_mensagem() {
    let cenario = Cenario::novo("erros", "transferir_agora");
    let arquivo = cenario.escrever_conjunto(CONJUNTO);

    let gate = cenario.poph(
        &["jev", "eval", "g10", arquivo.to_str().expect("caminho")],
        true,
    );
    assert_eq!(codigo(&gate), 1);
    assert!(texto(&gate.stderr).contains("desconhecido"));

    let ausente = cenario.poph(
        &["jev", "eval", "g1", "/caminho/que/nao/existe.jsonl"],
        true,
    );
    assert_eq!(codigo(&ausente), 1);
    assert!(texto(&ausente.stderr).contains("não consegui ler"));

    let json_ruim = cenario.escrever_conjunto("{ isto não é json }\n");
    let ruim = cenario.poph(
        &["jev", "eval", "g1", json_ruim.to_str().expect("caminho")],
        true,
    );
    assert_eq!(codigo(&ruim), 1);
    assert!(texto(&ruim.stderr).contains("caso inválido"));
}

#[test]
fn ajuda_lista_o_subcomando() {
    let cenario = Cenario::novo("ajuda", "transferir_agora");
    let saida = cenario.poph(&["--help"], false);
    let impresso = texto(&saida.stdout);
    assert!(impresso.contains("jev"), "saída: {impresso}");
    assert!(Path::new(&cenario.pasta.join("poph.toml")).exists());
}

/// Aceite da calibração: com resposta abaixo do limiar, o relatório mostra a escolha do modelo, a
/// confiança e o motivo do fallback — no texto e no JSON.
#[test]
fn abstencao_mostra_escolha_e_motivo() {
    // Confiança 0,50 contra o limiar 0,70 do G1: a resposta existe e é recusada.
    let cenario = Cenario::detalhado("abstencao", "transferir_agora", 0.5, Some((30, 4)));
    let arquivo = cenario.escrever_conjunto(CONJUNTO);

    let saida = cenario.poph(
        &["jev", "eval", "g1", arquivo.to_str().expect("caminho")],
        true,
    );
    assert_eq!(codigo(&saida), 0, "stderr: {}", texto(&saida.stderr));
    let impresso = texto(&saida.stdout);
    assert!(impresso.contains("detalhe por caso"), "saída: {impresso}");
    assert!(
        impresso.contains("jev transferir_agora"),
        "a escolha do modelo aparece no fallback: {impresso}"
    );
    assert!(
        impresso.contains("motivo: confiança 0.50 abaixo do limiar 0.70"),
        "saída: {impresso}"
    );
    assert!(
        impresso.contains("tokens: entrada 60 · saída 8 · total 68"),
        "usage somado das duas chamadas: {impresso}"
    );

    let saida = cenario.poph(
        &[
            "jev",
            "eval",
            "g1",
            arquivo.to_str().expect("caminho"),
            "--json",
        ],
        true,
    );
    assert_eq!(codigo(&saida), 0, "stderr: {}", texto(&saida.stderr));
    let valor: serde_json::Value =
        serde_json::from_str(&texto(&saida.stdout)).expect("saída é JSON");
    assert_eq!(valor["versao_relatorio"], 1);
    assert_eq!(valor["relatorio"]["abstencoes"], 2);
    assert_eq!(valor["relatorio"]["erros"], 0);
    assert_eq!(
        valor["relatorio"]["detalhes"][0]["escolha"],
        "transferir_agora"
    );
    assert_eq!(
        valor["relatorio"]["detalhes"][0]["obtido"],
        serde_json::Value::Null
    );
    assert!(
        valor["relatorio"]["detalhes"][0]["motivo"]
            .as_str()
            .unwrap_or_default()
            .contains("abaixo do limiar")
    );
    assert_eq!(valor["relatorio"]["usage"]["entrada"], 60);
    assert_eq!(valor["relatorio"]["usage"]["saida"], 8);
    assert_eq!(valor["relatorio"]["usage"]["total"], 68);
    assert_eq!(valor["relatorio"]["usage"]["informado"], true);
    assert!(!texto(&saida.stdout).contains(CHAVE));
}

/// Sem `usage` no corpo da resposta, o relatório diz "não informado" em vez de inventar número.
#[test]
fn sem_usage_diz_nao_informado() {
    let cenario = Cenario::detalhado("sem-usage", "continuar_agente", 0.9, None);
    let arquivo = cenario.escrever_conjunto(CONJUNTO);

    let saida = cenario.poph(
        &["jev", "eval", "g1", arquivo.to_str().expect("caminho")],
        true,
    );
    assert_eq!(codigo(&saida), 0, "stderr: {}", texto(&saida.stderr));
    assert!(
        texto(&saida.stdout).contains("tokens: não informado"),
        "saída: {}",
        texto(&saida.stdout)
    );

    let saida = cenario.poph(
        &[
            "jev",
            "eval",
            "g1",
            arquivo.to_str().expect("caminho"),
            "--json",
        ],
        true,
    );
    let valor: serde_json::Value =
        serde_json::from_str(&texto(&saida.stdout)).expect("saída é JSON");
    assert_eq!(valor["relatorio"]["usage"]["informado"], false);
    assert_eq!(
        valor["relatorio"]["usage"]["entrada"],
        serde_json::Value::Null
    );
}
