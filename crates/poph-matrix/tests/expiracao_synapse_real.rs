//! Expiração da mídia contra o Synapse **real de desenvolvimento** da MX-01 (MX-46).
//!
//! Fica `#[ignore]`: não roda no CI (que não tem Synapse) e precisa do ambiente de pé em
//! `~/.cache/poph-matrix/credenciais.env`, com um token de **administrador** do Synapse. Rodar com:
//!
//! ```text
//! bash apps/pop-harness/scripts/matrix-dev.sh subir
//! cargo test -p poph-matrix --test expiracao_synapse_real -- --ignored --nocapture
//! ```
//!
//! Existe porque nem o Synapse falso nem o banco provam o que o servidor de verdade faz com as duas
//! chamadas da expiração:
//!
//! - a **redação** feita por um moderador da sala em nome do fantasma do agente, e se o bloco
//!   próprio (`com.<marca>.attachment_expired`) sobrevive no `unsigned.redacted_because.content` —
//!   é de lá que o Element tira o aviso "não está mais disponível";
//! - o **apagamento pela API administrativa** (`DELETE /_synapse/admin/v1/media/{servidor}/{id}`),
//!   que só funciona com token de administrador: depois dele, a mídia não desce mais (404).
//!
//! O que este teste **não** faz: apagar mídia de cliente real. Os bytes são um PNG 1×1 de um teste,
//! numa sala criada por esta execução.
//!
//! **Variável necessária:** `POPH_MATRIX_DEV_ADMIN_TOKEN` em `credenciais.env`. Sem ela, o teste
//! falha dizendo o que falta — não inventa token nem finge que passou.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use poph_matrix::cliente::{ClienteMatrix, ConfigCliente};
use poph_matrix::expiracao::{
    self, BLOCO_EXPIRADO, CAMPO_CATEGORIA, CAMPO_ENVIADA_EM, CAMPO_NOME, MidiaParaExpirar,
};
use poph_matrix::midia::{self, AnexoParaMidia, BLOCO_ANEXO, LIMITE_MIDIA_BYTES};
use poph_matrix::salas::{
    ConversaParaSala, ErroPorta, EstadoGravado, PortaSalas, Provisionador, SalaGravada,
};
use poph_matrix::usuarios::IdsFantasmas;
use poph_types::{AgentId, ConversationId, MatrixConfig, OrgId, Segredo};
use serde_json::json;

/// Porta em memória: o teste quer provar o Synapse, não o banco.
#[derive(Clone, Default)]
struct PortaMemoria(Arc<Mutex<BTreeMap<(OrgId, ConversationId), SalaGravada>>>);

impl PortaSalas for PortaMemoria {
    async fn sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
    ) -> Result<Option<SalaGravada>, ErroPorta> {
        Ok(self.0.lock().expect("trava").get(&(org, conversa)).cloned())
    }

    async fn garantir_sala(
        &self,
        org: OrgId,
        conversa: ConversationId,
        room_id: &str,
    ) -> Result<SalaGravada, ErroPorta> {
        let mut guarda = self.0.lock().expect("trava");
        let sala = guarda
            .entry((org, conversa))
            .or_insert_with(|| SalaGravada {
                room_id: room_id.to_owned(),
                estado: EstadoGravado::Criando,
            });
        Ok(sala.clone())
    }

    async fn marcar_pronta(
        &self,
        org: OrgId,
        conversa: ConversationId,
        _nome_espelhado: Option<&str>,
    ) -> Result<SalaGravada, ErroPorta> {
        let mut guarda = self.0.lock().expect("trava");
        match guarda.get_mut(&(org, conversa)) {
            Some(sala) => {
                sala.estado = EstadoGravado::Pronta;
                Ok(sala.clone())
            }
            None => Err(ErroPorta::nova("sala ausente na porta")),
        }
    }

    async fn teve_mensagens_antes(
        &self,
        _org: OrgId,
        _conversa: ConversationId,
        _corte: i64,
    ) -> Result<bool, ErroPorta> {
        Ok(false)
    }
}

/// PNG 1×1 transparente: um arquivo de imagem de verdade, pequeno e sem dado de ninguém.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

/// Lê uma chave do `credenciais.env` sem imprimir valor nenhum.
fn credencial(nome: &str) -> String {
    let caminho = std::env::var("HOME").expect("HOME") + "/.cache/poph-matrix/credenciais.env";
    let conteudo = std::fs::read_to_string(&caminho).unwrap_or_else(|erro| {
        panic!(
            "não consegui ler {caminho} ({erro}): suba o ambiente com \
             `bash apps/pop-harness/scripts/matrix-dev.sh subir`"
        )
    });
    conteudo
        .lines()
        .filter_map(|linha| linha.split_once('='))
        .find(|(chave, _)| chave.trim() == nome)
        .map(|(_, valor)| valor.trim().trim_matches('"').to_owned())
        .unwrap_or_else(|| {
            panic!(
                "{nome} ausente em {caminho}. Para a MX-46 é preciso um usuário **administrador** do \
                 Synapse de desenvolvimento e o token de acesso dele em POPH_MATRIX_DEV_ADMIN_TOKEN."
            )
        })
}

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar, com token de administrador"]
async fn a_midia_expirada_e_redigida_na_sala_e_apagada_do_synapse() {
    let base = credencial("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credencial("POPH_MATRIX_DEV_SERVER_NAME");
    let as_token = credencial("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let admin_token = credencial("POPH_MATRIX_DEV_ADMIN_TOKEN");

    let config = MatrixConfig {
        habilitado: true,
        homeserver_url: base,
        server_name: server_name.clone(),
        prefixo: "ph".to_owned(),
        ..MatrixConfig::default()
    };
    let cliente = ClienteMatrix::novo(
        ConfigCliente::novo(
            config.homeserver_url.clone(),
            server_name.clone(),
            Segredo::new(as_token),
        )
        .expect("configuração do cliente"),
    )
    .expect("cliente");
    let ids = IdsFantasmas::novo("ph", server_name.clone()).expect("ids dos fantasmas");

    // Marca de tempo: cada execução cria a própria sala no ambiente descartável.
    let marca = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();
    let peer = format!("5521{marca}");
    let agente_id = AgentId::generate();
    let dados = ConversaParaSala {
        org: OrgId::generate(),
        conversa: ConversationId::generate(),
        cliente: peer.clone(),
        nome_cliente: Some(format!("Cliente MX-46 {marca}")),
        agente: agente_id,
        nome_agente: Some("Agente de Expiração".to_owned()),
        canal: "whatsapp".to_owned(),
        departamento: None,
        fechada: false,
        corte_backfill: None,
    };
    // Sala criada pelo caminho real (MX-11): registra os fantasmas, cria a sala e põe os dois
    // dentro. É o mesmo `Provisionador` que o servidor usa.
    let provisionador = Provisionador::da_config(cliente.clone(), PortaMemoria::default(), &config)
        .expect("provisionador");
    let pronta = provisionador
        .provisionar(&dados)
        .await
        .expect("sala criada contra o Synapse real");
    let sala = pronta.room_id.clone();
    let agente = ids
        .agente(&agente_id.to_string())
        .expect("fantasma do agente");
    let cliente_fantasma = ids.cliente(&peer).expect("fantasma do cliente");

    // O cliente manda a imagem (MX-43): sobe o arquivo e publica o evento.
    let nome = format!("foto-{marca}.png");
    let decidida = midia::decidir(&AnexoParaMidia {
        id: &format!("anx-mx46-{marca}"),
        nome: &nome,
        mime: "image/png",
        tamanho_bytes: PNG.len() as u64,
        fonte: "whatsapp",
    })
    .expect("decide a mídia");
    let publicada = midia::enviar(&cliente, &cliente_fantasma, &sala, &decidida, PNG)
        .await
        .expect("publica a mídia na sala");

    // O Synapse de verdade devolveu `mxc://`, e o evento está lá com o bloco do anexo.
    assert!(
        publicada.mxc.starts_with("mxc://"),
        "a subida devolveu um mxc: {}",
        publicada.mxc
    );
    let antes = cliente
        .ler_evento(cliente_fantasma.user_id(), &sala, &publicada.evento)
        .await
        .expect("lê o evento publicado");
    assert!(antes["content"][BLOCO_ANEXO]["id"].is_string());
    let bytes_antes = cliente
        .baixar_midia(
            cliente_fantasma.user_id(),
            &publicada.mxc,
            LIMITE_MIDIA_BYTES,
        )
        .await
        .expect("antes de expirar, a mídia desce");
    assert_eq!(bytes_antes, PNG, "os bytes sobem e descem inteiros");

    // ── expiração: redigir (como o fantasma do agente) e apagar (API admin) ──
    let para_expirar = MidiaParaExpirar {
        id: &decidida.id_anexo,
        nome: &nome,
        mime: "image/png",
        categoria: "image",
        tamanho_bytes: PNG.len() as u64,
        enviada_em: 1_797_900_000_000,
    };
    expiracao::redigir(&cliente, &agente, &sala, &publicada.evento, &para_expirar)
        .await
        .expect("o Synapse aceita a redação");

    // O que o Element lê: o bloco da expiração dentro de `unsigned.redacted_because.content`.
    let depois = cliente
        .ler_evento(agente.user_id(), &sala, &publicada.evento)
        .await
        .expect("lê o evento redigido");
    let redacao = &depois["unsigned"]["redacted_because"];
    assert_eq!(
        redacao["sender"],
        json!(agente.user_id()),
        "a redação foi assinada pelo fantasma do agente"
    );
    let bloco = &redacao["content"][BLOCO_EXPIRADO];
    assert_eq!(
        bloco[CAMPO_NOME],
        json!(nome),
        "o aviso mostra o nome do arquivo"
    );
    assert_eq!(bloco[CAMPO_CATEGORIA], json!("image"));
    assert_eq!(bloco[CAMPO_ENVIADA_EM], json!("2026-12-22T00:40:00Z"));
    assert_eq!(bloco["mimetype"], json!("image/png"));
    assert_eq!(bloco["size"], json!(PNG.len() as u64));
    assert!(
        depois["content"]["url"].is_null(),
        "o conteúdo do evento foi apagado pela redação: {}",
        depois["content"]
    );

    // Repetir a redação não muda o que o cliente vê (mesmo `txnId`, e a primeira vale).
    expiracao::redigir(&cliente, &agente, &sala, &publicada.evento, &para_expirar)
        .await
        .expect("a segunda redação do mesmo evento não é erro");
    let de_novo = cliente
        .ler_evento(agente.user_id(), &sala, &publicada.evento)
        .await
        .expect("lê de novo");
    assert_eq!(
        de_novo["unsigned"]["redacted_because"]["content"][BLOCO_EXPIRADO],
        redacao["content"][BLOCO_EXPIRADO],
        "o aviso continua o mesmo"
    );

    // Apagar pela API administrativa: com o token de administrador, o arquivo sai.
    let apagou = expiracao::apagar(&cliente, &Segredo::new(admin_token), &publicada.mxc)
        .await
        .expect("o Synapse de dev aceita o apagamento pela API admin");
    assert!(apagou, "o arquivo existia e foi apagado");

    // Depois do apagamento a mídia não desce mais (404), e repetir conta como já apagada.
    let erro = cliente
        .baixar_midia(
            cliente_fantasma.user_id(),
            &publicada.mxc,
            LIMITE_MIDIA_BYTES,
        )
        .await
        .expect_err("mídia apagada não desce");
    println!("depois do apagamento, o download responde: {erro}");

    println!(
        "MX-46 no Synapse real: evento {} redigido na sala {sala} e arquivo {} apagado",
        publicada.evento, publicada.mxc
    );
    // O `client_token` do admin não vai para log: o teste só usa para apagar.
    println!("servidor: {server_name}");
}
