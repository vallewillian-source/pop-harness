use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use poph_matrix::client::{ClientConfig, CreateRoom, MatrixClient};
use poph_matrix::rooms::PortError;
use poph_matrix::spaces::{
    CHILD_KIND, DepartmentSpace, InviteLimits, PARENT_KIND, Position, RoomForSpace, SpaceOrigin,
    Spaces, SpacesPort, StoredSpace,
};
use poph_matrix::users::GhostIds;
use poph_types::{ConversationId, DepartmentId, OrgId, Secret};
use serde_json::json;

fn credential(name: &str) -> String {
    let path = std::env::var("HOME").expect("HOME") + "/.cache/poph-matrix/credenciais.env";
    let content = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "não consegui ler {path} ({error}): suba o ambiente com \
             `bash apps/pop-harness/scripts/matrix-dev.sh subir`"
        )
    });
    content
        .lines()
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().trim_matches('"').to_owned())
        .unwrap_or_else(|| panic!("{name} ausente em {path}"))
}

#[derive(Clone, Default)]
struct MemoryPort(Arc<Mutex<BTreeMap<String, String>>>);

impl SpacesPort for MemoryPort {
    async fn space(
        &self,
        _org: OrgId,
        department: DepartmentId,
    ) -> Result<Option<StoredSpace>, PortError> {
        let guard = self.0.lock().expect("trava");
        Ok(guard
            .get(&format!("espaco:{department}"))
            .map(|value| StoredSpace {
                room_id: value.clone(),
                alias: String::new(),
                origin: SpaceOrigin::Automatic,
            }))
    }

    async fn ensure_space(
        &self,
        _org: OrgId,
        department: DepartmentId,
        room_id: &str,
        _alias: &str,
        origin: SpaceOrigin,
    ) -> Result<StoredSpace, PortError> {
        let mut guard = self.0.lock().expect("trava");
        let current = guard
            .entry(format!("espaco:{department}"))
            .or_insert_with(|| room_id.to_owned())
            .clone();
        Ok(StoredSpace {
            room_id: current,
            alias: String::new(),
            origin,
        })
    }

    async fn room_space(
        &self,
        _org: OrgId,
        conversation: ConversationId,
    ) -> Result<Option<String>, PortError> {
        Ok(self
            .0
            .lock()
            .expect("trava")
            .get(&format!("sala:{conversation}"))
            .cloned())
    }

    async fn set_room_space(
        &self,
        _org: OrgId,
        conversation: ConversationId,
        space: Option<&str>,
    ) -> Result<(), PortError> {
        let mut guard = self.0.lock().expect("trava");
        match space {
            Some(space) => {
                guard.insert(format!("sala:{conversation}"), space.to_owned());
            }
            None => {
                guard.remove(&format!("sala:{conversation}"));
            }
        }
        Ok(())
    }
}

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar"]
async fn room_enters_department_space_and_changes_space() {
    let base = credential("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credential("POPH_MATRIX_DEV_SERVER_NAME");
    let token = credential("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let config = poph_types::MatrixConfig {
        enabled: true,
        homeserver_url: base,
        server_name: server_name.clone(),
        prefix: "ph".to_owned(),
        ..poph_types::MatrixConfig::default()
    };
    let client = MatrixClient::new(
        ClientConfig::new(
            config.homeserver_url.clone(),
            server_name.clone(),
            Secret::new(token),
        )
        .expect("configuração do cliente"),
    )
    .expect("cliente");
    let bot = format!("@ph_as_bot:{server_name}");
    let marker = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();

    let room = client
        .create_room(
            &bot,
            &CreateRoom {
                room_alias_name: Some(format!("ph_c_teste12_{marker}")),
                name: Some(format!("Sala MX-12 {marker}")),
                ..CreateRoom::default()
            },
        )
        .await
        .expect("createRoom da sala");
    println!("sala: {room}");

    let ids = GhostIds::new("ph", server_name.clone()).expect("ids");
    let spaces = Spaces::with(client.clone(), ids, MemoryPort::default())
        .with_limits(InviteLimits::with(5, 2, Duration::ZERO, 10));

    let commercial = DepartmentSpace {
        id: DepartmentId::generate(),
        name: format!("Comercial {marker}"),
    };
    let givens = RoomForSpace {
        org: OrgId::generate(),
        conversation: ConversationId::generate(),
        room: room.clone(),
        agent: client_agent(&server_name),
        department: Some(commercial.clone()),
        existing: None,
    };

    let position = spaces.place(&givens).await.expect("posicionar");
    let Position::Positioned { space, .. } = position else {
        panic!("esperava Posicionada, veio {position:?}");
    };
    println!("espaço do departamento: {space}");

    let children = client
        .hierarquia(&bot, &space)
        .await
        .expect("hierarchy do espaço");
    println!("salas sob o espaço: {children:?}");
    assert!(
        children.contains(&room),
        "a sala tem de aparecer sob o espaço do departamento"
    );
    let parent = client
        .read_state(&bot, &room, PARENT_KIND, Some(&space))
        .await
        .expect("m.space.parent");
    println!("m.space.parent -> {parent}");
    assert_eq!(parent["canonical"], json!(true));

    let repeated = spaces.place(&givens).await.expect("repetir");
    assert_eq!(
        repeated,
        Position::AlreadyPositioned {
            space: space.clone()
        }
    );

    let support = DepartmentSpace {
        id: DepartmentId::generate(),
        name: format!("Suporte {marker}"),
    };
    let mut transferred = givens.clone();
    transferred.department = Some(support);
    let position = spaces.place(&transferred).await.expect("mover");
    let Position::Positioned {
        space: new, moved, ..
    } = position
    else {
        panic!("esperava Posicionada");
    };
    assert!(moved, "a sala veio de outro espaço");
    println!("espaço novo: {new}");
    let children = client
        .hierarquia(&bot, &new)
        .await
        .expect("hierarchy do espaço novo");
    assert!(children.contains(&room), "a sala está sob o espaço novo");
    let old = client
        .read_state(&bot, &space, CHILD_KIND, Some(&room))
        .await
        .expect("m.space.child do espaço antigo");
    println!("m.space.child antigo -> {old}");
    assert_eq!(
        old,
        json!({}),
        "o filho do espaço antigo ficou vazio (removido)"
    );

    println!("espaços por departamento provados no Synapse de desenvolvimento");
}

fn client_agent(server_name: &str) -> poph_matrix::users::Ghost {
    GhostIds::new("ph", server_name).expect("ids").bot()
}

#[tokio::test]
#[ignore = "precisa do Synapse de desenvolvimento da MX-01 no ar"]
async fn room_without_record_leaves_old_space_in_real_synapse() {
    let base = credential("POPH_MATRIX_DEV_BASE_URL");
    let server_name = credential("POPH_MATRIX_DEV_SERVER_NAME");
    let token = credential("POPH_MATRIX_DEV_HARNESS_AS_TOKEN");
    let client = MatrixClient::new(
        ClientConfig::new(base, server_name.clone(), Secret::new(token))
            .expect("configuração do cliente"),
    )
    .expect("cliente");
    let bot = format!("@ph_as_bot:{server_name}");
    let marker = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();

    let room = client
        .create_room(
            &bot,
            &CreateRoom {
                room_alias_name: Some(format!("ph_c_mx51_{marker}")),
                name: Some(format!("Sala MX-51 {marker}")),
                ..CreateRoom::default()
            },
        )
        .await
        .expect("createRoom da sala");

    let ids = GhostIds::new("ph", server_name.clone()).expect("ids");
    let port = MemoryPort::default();
    let spaces = Spaces::with(client.clone(), ids, port.clone()).with_limits(InviteLimits::with(
        5,
        2,
        Duration::ZERO,
        10,
    ));

    let mut givens = RoomForSpace {
        org: OrgId::generate(),
        conversation: ConversationId::generate(),
        room: room.clone(),
        agent: client_agent(&server_name),
        department: Some(DepartmentSpace {
            id: DepartmentId::generate(),
            name: format!("Comercial MX51 {marker}"),
        }),
        existing: None,
    };
    let Position::Positioned { space: old, .. } =
        spaces.place(&givens).await.expect("no comercial")
    else {
        panic!("esperava Posicionada");
    };

    port.set_room_space(givens.org, givens.conversation, None)
        .await
        .expect("registro vazio");
    givens.department = Some(DepartmentSpace {
        id: DepartmentId::generate(),
        name: format!("Suporte MX51 {marker}"),
    });
    let fresh = spaces
        .place(&givens)
        .await
        .expect("transferência com registro vazio");
    let Position::Positioned {
        space: new,
        moved: true,
        ..
    } = fresh
    else {
        panic!("esperava a sala movida, veio {fresh:?}");
    };

    let under_old = client
        .hierarquia(&bot, &old)
        .await
        .expect("hierarchy do antigo");
    assert!(
        !under_old.contains(&room),
        "a sala não pode continuar sob o espaço antigo"
    );
    let under_new = client
        .hierarquia(&bot, &new)
        .await
        .expect("hierarchy do novo");
    assert!(
        under_new.contains(&room),
        "a sala tem de estar sob o espaço novo"
    );
    let old_parent = client
        .read_state(&bot, &room, PARENT_KIND, Some(&old))
        .await
        .expect("m.space.parent do antigo");
    assert_eq!(
        old_parent,
        json!({}),
        "o pai do espaço antigo foi tirado da sala"
    );
    println!("sala sem registro saiu do espaço antigo no Synapse de desenvolvimento");
}
