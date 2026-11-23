//! Teste dos espaços por departamento contra o Synapse **real de desenvolvimento** da MX-01 (MX-12).
//!
//! Fica `#[ignore]`: não roda no CI (que não tem Synapse) e precisa do ambiente de pé em
//! `~/.cache/poph-matrix/credenciais.env`. Rodar com:
//!
//! ```text
//! bash apps/pop-harness/scripts/matrix-dev.sh subir
//! cargo test -p poph-matrix --test espacos_synapse_real -- --ignored --nocapture
//! ```
//!
//! O teste cria um espaço, uma sala e um departamento com marcação de tempo no nome; é ambiente
//! descartável. A persistência é um mapa em memória: aqui interessa o que o Synapse faz com os
//! eventos, não o banco.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use poph_matrix::cliente::{ClienteMatrix, ConfigCliente, CriarSala};
use poph_matrix::espacos::{
    DepartamentoEspaco, EspacoGravado, Espacos, LimitesConvite, OrigemEspaco, PortaEspacos,
    Posicao, SalaParaEspaco, TIPO_FILHO, TIPO_PAI,
};
use poph_matrix::salas::ErroPorta;
use poph_matrix::usuarios::IdsFantasmas;
use poph_types::{ConversationId, DepartmentId, OrgId, Segredo};
use serde_json::json;

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
struct PortaMemoria(Arc<Mutex<BTreeMap<String, String>>>);

impl PortaEspacos for PortaMemoria {
    async fn espaco(
        &self,
        _org: OrgId,
        departamento: DepartmentId,
    ) -> Result<Option<EspacoGravado>, ErroPorta> {
        let guarda = self.0.lock().expect("trava");
        Ok(guarda
            .get(&format!("espaco:{departamento}"))
            .map(|valor| EspacoGravado {
                room_id: valor.clone(),
                alias: String::new(),
                origem: OrigemEspaco::Automatico,
            }))
    }

    async fn garantir_espaco(
        &self,
        _org: OrgId,
        departamento: DepartmentId,
        room_id: &str,
        _alias: &str,
        origem: OrigemEspaco,
    ) -> Result<EspacoGravado, ErroPorta> {
        let mut guarda = self.0.lock().expect("trava");
        let atual = guarda
            .entry(format!("espaco:{departamento}"))
            .or_insert_with(|| room_id.to_owned())
            .clone();
        Ok(EspacoGravado {
            room_id: atual,
            alias: String::new(),
            origem,
        })
    }

    async fn espaco_da_sala(
        &self,
        _org: OrgId,
        conversa: ConversationId,
    ) -> Result<Option<String>, ErroPorta> {
        Ok(self
            .0
            .lock()
            .expect("trava")
            .get(&format!("sala:{conversa}"))
            .cloned())
    }

    async fn definir_espaco_da_sala(
        &self,
        _org: OrgId,
        conversa: ConversationId,
        espaco: Option<&str>,
    ) -> Result<(), ErroPorta> {
        let mut guarda = self.0.lock().expect("trava");
        match espaco {
            Some(espaco) => {
                guarda.insert(format!("sala:{conversa}"), espaco.to_owned());
            }
            None => {
                guarda.remove(&format!("sala:{conversa}"));
            }
        }
        Ok(())
    }
}

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar"]
async fn a_sala_entra_no_espaco_do_departamento_e_muda_de_espaco() {
    let base = credencial("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credencial("POPH_MATRIX_DEV_SERVER_NAME");
    let token = credencial("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let config = poph_types::MatrixConfig {
        habilitado: true,
        homeserver_url: base,
        server_name: server_name.clone(),
        prefixo: "ph".to_owned(),
        ..poph_types::MatrixConfig::default()
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
    let bot = format!("@ph_as_bot:{server_name}");
    let marcador = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();

    let sala = cliente
        .criar_sala(
            &bot,
            &CriarSala {
                room_alias_name: Some(format!("ph_c_teste12_{marcador}")),
                name: Some(format!("Sala MX-12 {marcador}")),
                ..CriarSala::default()
            },
        )
        .await
        .expect("createRoom da sala");
    println!("sala: {sala}");

    let ids = IdsFantasmas::novo("ph", server_name.clone()).expect("ids");
    let espacos = Espacos::com(cliente.clone(), ids, PortaMemoria::default())
        .com_limites(LimitesConvite::com(5, 2, Duration::ZERO, 10));

    let comercial = DepartamentoEspaco {
        id: DepartmentId::generate(),
        nome: format!("Comercial {marcador}"),
    };
    let dados = SalaParaEspaco {
        org: OrgId::generate(),
        conversa: ConversationId::generate(),
        sala: sala.clone(),
        agente: cliente_agente(&server_name),
        departamento: Some(comercial.clone()),
        existente: None,
    };

    let posicao = espacos.posicionar(&dados).await.expect("posicionar");
    let Posicao::Posicionada { espaco, .. } = posicao else {
        panic!("esperava Posicionada, veio {posicao:?}");
    };
    println!("espaço do departamento: {espaco}");

    let filhas = cliente
        .hierarquia(&bot, &espaco)
        .await
        .expect("hierarchy do espaço");
    println!("salas sob o espaço: {filhas:?}");
    assert!(
        filhas.contains(&sala),
        "a sala tem de aparecer sob o espaço do departamento"
    );
    let pai = cliente
        .ler_estado(&bot, &sala, TIPO_PAI, Some(&espaco))
        .await
        .expect("m.space.parent");
    println!("m.space.parent -> {pai}");
    assert_eq!(pai["canonical"], json!(true));

    // Repetir não cria segundo espaço (a linha gravada manda).
    let repetida = espacos.posicionar(&dados).await.expect("repetir");
    assert_eq!(
        repetida,
        Posicao::JaPosicionada {
            espaco: espaco.clone()
        }
    );

    // Transferência para outro departamento: a sala muda de espaço.
    let suporte = DepartamentoEspaco {
        id: DepartmentId::generate(),
        nome: format!("Suporte {marcador}"),
    };
    let mut transferida = dados.clone();
    transferida.departamento = Some(suporte);
    let posicao = espacos.posicionar(&transferida).await.expect("mover");
    let Posicao::Posicionada {
        espaco: novo,
        movida,
        ..
    } = posicao
    else {
        panic!("esperava Posicionada");
    };
    assert!(movida, "a sala veio de outro espaço");
    println!("espaço novo: {novo}");
    let filhas = cliente
        .hierarquia(&bot, &novo)
        .await
        .expect("hierarchy do espaço novo");
    assert!(filhas.contains(&sala), "a sala está sob o espaço novo");
    let antigo = cliente
        .ler_estado(&bot, &espaco, TIPO_FILHO, Some(&sala))
        .await
        .expect("m.space.child do espaço antigo");
    println!("m.space.child antigo -> {antigo}");
    assert_eq!(
        antigo,
        json!({}),
        "o filho do espaço antigo ficou vazio (removido)"
    );

    println!("espaços por departamento provados no Synapse de desenvolvimento");
}

/// Fantasma do agente para o teste: o próprio bot (aqui não há agente de verdade no banco).
fn cliente_agente(server_name: &str) -> poph_matrix::usuarios::Fantasma {
    IdsFantasmas::novo("ph", server_name).expect("ids").bot()
}

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar"]
async fn a_sala_sem_registro_sai_do_espaco_antigo_no_synapse_real() {
    // MX-51: a transferência chega com o registro da conversa vazio (a janela da criação). A sala
    // tem de sair do espaço antigo mesmo assim, achando-o pelo `m.space.parent` dela.
    let base = credencial("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credencial("POPH_MATRIX_DEV_SERVER_NAME");
    let token = credencial("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let cliente = ClienteMatrix::novo(
        ConfigCliente::novo(base, server_name.clone(), Segredo::new(token))
            .expect("configuração do cliente"),
    )
    .expect("cliente");
    let bot = format!("@ph_as_bot:{server_name}");
    let marcador = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();

    let sala = cliente
        .criar_sala(
            &bot,
            &CriarSala {
                room_alias_name: Some(format!("ph_c_mx51_{marcador}")),
                name: Some(format!("Sala MX-51 {marcador}")),
                ..CriarSala::default()
            },
        )
        .await
        .expect("createRoom da sala");

    let ids = IdsFantasmas::novo("ph", server_name.clone()).expect("ids");
    let porta = PortaMemoria::default();
    let espacos = Espacos::com(cliente.clone(), ids, porta.clone())
        .com_limites(LimitesConvite::com(5, 2, Duration::ZERO, 10));

    let mut dados = SalaParaEspaco {
        org: OrgId::generate(),
        conversa: ConversationId::generate(),
        sala: sala.clone(),
        agente: cliente_agente(&server_name),
        departamento: Some(DepartamentoEspaco {
            id: DepartmentId::generate(),
            nome: format!("Comercial MX51 {marcador}"),
        }),
        existente: None,
    };
    let Posicao::Posicionada { espaco: antigo, .. } =
        espacos.posicionar(&dados).await.expect("no comercial")
    else {
        panic!("esperava Posicionada");
    };

    porta
        .definir_espaco_da_sala(dados.org, dados.conversa, None)
        .await
        .expect("registro vazio");
    dados.departamento = Some(DepartamentoEspaco {
        id: DepartmentId::generate(),
        nome: format!("Suporte MX51 {marcador}"),
    });
    let nova = espacos
        .posicionar(&dados)
        .await
        .expect("transferência com registro vazio");
    let Posicao::Posicionada {
        espaco: novo,
        movida: true,
        ..
    } = nova
    else {
        panic!("esperava a sala movida, veio {nova:?}");
    };

    let sob_antigo = cliente
        .hierarquia(&bot, &antigo)
        .await
        .expect("hierarchy do antigo");
    assert!(
        !sob_antigo.contains(&sala),
        "a sala não pode continuar sob o espaço antigo"
    );
    let sob_novo = cliente
        .hierarquia(&bot, &novo)
        .await
        .expect("hierarchy do novo");
    assert!(
        sob_novo.contains(&sala),
        "a sala tem de estar sob o espaço novo"
    );
    let pai_antigo = cliente
        .ler_estado(&bot, &sala, TIPO_PAI, Some(&antigo))
        .await
        .expect("m.space.parent do antigo");
    assert_eq!(
        pai_antigo,
        json!({}),
        "o pai do espaço antigo foi tirado da sala"
    );
    println!("sala sem registro saiu do espaço antigo no Synapse de desenvolvimento");
}
