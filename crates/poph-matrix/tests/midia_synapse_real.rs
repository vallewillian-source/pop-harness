//! Mídia do cliente contra o Synapse **real de desenvolvimento** da MX-01 (MX-43, fase 2).
//!
//! Fica `#[ignore]`: não roda no CI (que não tem Synapse) e precisa do ambiente de pé em
//! `~/.cache/poph-matrix/credenciais.env`. Rodar com:
//!
//! ```text
//! bash apps/pop-harness/scripts/matrix-dev.sh subir
//! cargo test -p poph-matrix --test midia_synapse_real -- --ignored --nocapture
//! ```
//!
//! Existe porque o Synapse falso não prova o que o servidor de verdade faz com a subida de arquivo
//! feita em nome de um usuário virtual: se o `?user_id=` não vale na rota de mídia, se o
//! `Content-Type` é guardado, se o `mxc://` devolvido é servível e se o evento aceita o bloco extra.
//! O que este teste confere é o que o atendente vê no Element: um `m.image` e um `m.file` do
//! fantasma **do cliente**, com o bloco `com.<marca>.attachment` e o `mxc://` no conteúdo.
//!
//! A persistência é um mapa em memória: aqui interessa o que o Synapse faz com os eventos.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use poph_matrix::cliente::{ClienteMatrix, ConfigCliente};
use poph_matrix::midia::{self, AnexoParaMidia, BLOCO_ANEXO, MSGTYPE_ARQUIVO, MSGTYPE_IMAGEM};
use poph_matrix::salas::{
    ConversaParaSala, ErroPorta, EstadoGravado, PortaSalas, Provisionador, SalaGravada,
};
use poph_types::{AgentId, ConversationId, MatrixConfig, OrgId, Segredo};

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
        .unwrap_or_else(|| panic!("{nome} ausente em {caminho}"))
}

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

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar"]
async fn imagem_e_pdf_do_cliente_aparecem_na_sala_do_synapse_real() {
    let base = credencial("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credencial("POPH_MATRIX_DEV_SERVER_NAME");
    let token = credencial("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
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
            Segredo::new(token),
        )
        .expect("configuração do cliente"),
    )
    .expect("cliente");
    let provisionador = Provisionador::da_config(cliente.clone(), PortaMemoria::default(), &config)
        .expect("provisionador");

    // Marca de tempo no identificador: cada execução cria uma sala nova no ambiente descartável.
    let marca = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();
    let peer = format!("5521{marca}");
    let dados = ConversaParaSala {
        org: OrgId::generate(),
        conversa: ConversationId::generate(),
        cliente: peer.clone(),
        nome_cliente: Some(format!("Cliente MX-43 {marca}")),
        agente: AgentId::generate(),
        nome_agente: Some("Agente de Teste".to_owned()),
        canal: "whatsapp".to_owned(),
        departamento: None,
        fechada: false,
        corte_backfill: None,
    };
    let pronta = provisionador
        .provisionar(&dados)
        .await
        .expect("sala criada contra o Synapse real");
    let sala = pronta.room_id.clone();
    let ids = poph_matrix::usuarios::IdsFantasmas::novo("ph", server_name.clone()).expect("ids");
    let remetente = ids.cliente(&peer).expect("fantasma do cliente");

    // PDF mínimo: conteúdo qualquer, o Synapse não interpreta arquivo.
    let pdf =
        b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF\n"
            .to_vec();

    let mut eventos = Vec::new();
    for (nome, mime, bytes) in [
        ("foto.png", "image/png", PNG.to_vec()),
        ("laudo.pdf", "application/pdf", pdf),
    ] {
        let decidida = midia::decidir(&AnexoParaMidia {
            id: &format!("anx-mx43-{marca}-{nome}"),
            nome,
            mime,
            tamanho_bytes: bytes.len() as u64,
            fonte: "whatsapp",
        })
        .expect("decide a mídia");
        let evento = midia::enviar(&cliente, &remetente, &sala, &decidida, &bytes)
            .await
            .unwrap_or_else(|erro| panic!("[{nome}] o Synapse real recusou a mídia ({erro:?})"));
        let lido = cliente
            .ler_evento(remetente.user_id(), &sala, &evento)
            .await
            .unwrap_or_else(|erro| panic!("[{nome}] o evento não está na sala ({erro:?})"));
        eventos.push((nome, lido));
    }

    for (nome, evento) in &eventos {
        assert_eq!(
            evento["type"],
            serde_json::json!("m.room.message"),
            "[{nome}] tipo do evento"
        );
        assert_eq!(
            evento["sender"],
            serde_json::json!(remetente.user_id()),
            "[{nome}] quem assina é o fantasma do cliente"
        );
        let conteudo = &evento["content"];
        let esperado = if *nome == "foto.png" {
            MSGTYPE_IMAGEM
        } else {
            MSGTYPE_ARQUIVO
        };
        assert_eq!(conteudo["msgtype"], serde_json::json!(esperado), "[{nome}]");
        assert_eq!(conteudo["body"], serde_json::json!(nome), "[{nome}]");
        assert!(
            conteudo["url"]
                .as_str()
                .is_some_and(|url| url.starts_with("mxc://")),
            "[{nome}] mxc:// no conteúdo: {}",
            conteudo["url"]
        );
        assert!(
            conteudo[BLOCO_ANEXO]["id"].is_string(),
            "[{nome}] bloco do anexo com id: {}",
            conteudo[BLOCO_ANEXO]
        );
    }

    // A imagem sai como `m.image` e o PDF como `m.file`: dois tipos, um de cada.
    assert_eq!(
        eventos[0].1["content"]["msgtype"],
        serde_json::json!(MSGTYPE_IMAGEM)
    );
    assert_eq!(
        eventos[1].1["content"]["msgtype"],
        serde_json::json!(MSGTYPE_ARQUIVO)
    );
    println!(
        "MX-43 no Synapse real: imagem e PDF na sala {sala} pelo fantasma {}",
        remetente.user_id()
    );
}
