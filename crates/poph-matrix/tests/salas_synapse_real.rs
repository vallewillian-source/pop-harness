//! Teste da criação de sala contra o Synapse **real de desenvolvimento** da MX-01 (MX-31, item 1).
//!
//! Fica `#[ignore]`: não roda no CI (que não tem Synapse) e precisa do ambiente de pé em
//! `~/.cache/poph-matrix/credenciais.env`. Rodar com:
//!
//! ```text
//! bash apps/pop-harness/scripts/matrix-dev.sh subir
//! cargo test -p poph-matrix --test salas_synapse_real -- --ignored --nocapture
//! ```
//!
//! Existe porque o Synapse falso não reproduzia a regra de poder da criação: o `createRoom` com o
//! criador abaixo de 100 devolve `403 M_FORBIDDEN` e deixa a sala criada pela metade (sem
//! `m.room.name`, sem `m.room.topic` e sem o metadata). O que este teste confere é o que o
//! atendente vê no Element: nome, tópico, metadata e o poder final do fantasma do agente.
//!
//! A persistência é um mapa em memória: aqui interessa o que o Synapse faz com os eventos, não o
//! banco.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use poph_matrix::cliente::{ClienteMatrix, ConfigCliente};
use poph_matrix::salas::{
    ConversaParaSala, ErroPorta, EstadoGravado, PODER_MINIMO_AGENTE, PortaSalas, Provisionador,
    SalaGravada, TIPO_METADATA,
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

/// Porta em memória: o teste quer provar o Synapse, não o banco (que tem teste próprio).
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

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar"]
async fn sala_nasce_com_nome_topico_metadata_e_poder_final_do_agente() {
    let base = credencial("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credencial("POPH_MATRIX_DEV_SERVER_NAME");
    let token = credencial("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let config = MatrixConfig {
        habilitado: true,
        homeserver_url: base,
        server_name: server_name.clone(),
        prefixo: "ph".to_owned(),
        // O poder final configurado: a criação usa 100 e desce para este valor.
        poder_agente: PODER_MINIMO_AGENTE,
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

    // Marca de tempo no nome: cada execução cria uma sala nova no ambiente descartável.
    let marca = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();
    let nome_cliente = format!("Cliente MX-31 {marca}");
    let dados = ConversaParaSala {
        org: OrgId::generate(),
        conversa: ConversationId::generate(),
        cliente: format!("5511{marca}"),
        nome_cliente: Some(nome_cliente.clone()),
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
    let sala = &pronta.room_id;
    let agente = format!("@ph_ag_{}:{server_name}", dados.agente);

    // `m.room.name`: sem o poder de criação correto o Synapse responde 404 aqui (a sala nasceu
    // sem nome) — é o defeito que derrubou o A1 da MX-31.
    let nome = cliente
        .ler_estado(&agente, sala, "m.room.name", None)
        .await
        .expect("a sala precisa ter nome contra o Synapse real");
    assert_eq!(nome["name"], serde_json::json!(nome_cliente));

    let topico = cliente
        .ler_estado(&agente, sala, "m.room.topic", None)
        .await
        .expect("a sala precisa ter tópico");
    assert_eq!(topico["topic"], serde_json::json!("Atendimento · whatsapp"));

    let metadata = cliente
        .ler_estado(&agente, sala, TIPO_METADATA, None)
        .await
        .expect("a sala precisa ter o metadata do frontend");
    assert_eq!(metadata["client_name"], serde_json::json!(nome_cliente));

    // O poder final vale: o fantasma do agente tem `matrix.poder_agente`, não o poder da criação.
    let poderes = cliente
        .ler_estado(&agente, sala, "m.room.power_levels", None)
        .await
        .expect("a sala precisa ter poderes");
    assert_eq!(
        poderes["users"][&agente],
        serde_json::json!(PODER_MINIMO_AGENTE),
        "o poder do agente desce para o configurado depois da criação"
    );
}
