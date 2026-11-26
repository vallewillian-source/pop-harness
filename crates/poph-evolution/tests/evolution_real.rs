use std::time::{Duration, SystemTime, UNIX_EPOCH};

use poph_evolution::{ConfigEvolution, EvolutionClient, EvolutionError};
use poph_types::Secret;

fn configure(url: &str, chave: &str, instancia: &str) -> EvolutionClient {
    EvolutionClient::new(
        ConfigEvolution::new(url, instancia, Secret::new(chave.to_owned()))
            .expect("configuração")
            .with_timeout(Duration::from_secs(20)),
    )
    .expect("cliente")
}

#[tokio::test]
#[ignore = "precisa da Evolution isolada no ambiente (POPH_EVOLUTION_REAL_URL/_CHAVE)"]
async fn instance_cycle_without_number_against_the_real_evolution() {
    let url = std::env::var("POPH_EVOLUTION_REAL_URL").expect("POPH_EVOLUTION_REAL_URL");
    let chave = std::env::var("POPH_EVOLUTION_REAL_CHAVE").expect("POPH_EVOLUTION_REAL_CHAVE");
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();
    let instancia = format!("poph-teste-cliente-{now_secs}");
    let client = configure(&url, &chave, &instancia);

    let bad_client = configure(&url, "chave-errada-de-proposito", &instancia);
    let error = bad_client.list_instances().await.expect_err("chave errada");
    assert!(
        matches!(
            error,
            EvolutionError::Response {
                status: 401 | 403,
                ..
            }
        ),
        "erro inesperado: {error:?}"
    );
    assert!(!format!("{error} {error:?}").contains("chave-errada-de-proposito"));

    client.create_instance().await.expect("criar instância");
    let verification: Result<(), String> = async {
        let pairing = client
            .connect(None)
            .await
            .map_err(|e| format!("connect sem número: {e}"))?;
        if !pairing
            .qr_base64
            .as_deref()
            .is_some_and(|qr| qr.starts_with("data:image"))
        {
            return Err("sem imagem do QR".to_owned());
        }
        if pairing.pairing_code.is_some() {
            return Err("código de pareamento veio sem número".to_owned());
        }
        let estado = client
            .connection_state()
            .await
            .map_err(|e| format!("estado: {e}"))?;
        if estado == "open" {
            return Err("instância sem aparelho não pode estar aberta".to_owned());
        }
        let instances = client
            .list_instances()
            .await
            .map_err(|e| format!("listar: {e}"))?;
        if !instances.to_string().contains(&instancia) {
            return Err("instância não apareceu na lista".to_owned());
        }
        println!("estado={estado}");
        Ok(())
    }
    .await;
    let deleted = client.delete().await;
    verification.expect("verificação contra a Evolution real");
    deleted.expect("apagar instância de teste");
}
