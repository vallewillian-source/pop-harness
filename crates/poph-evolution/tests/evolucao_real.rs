//! Cliente contra uma EvolutionAPI de verdade, isolada (HR-26a). **Ignorado por padrão.**
//!
//! Só roda com as duas variáveis no ambiente, e nunca toca número de WhatsApp (cria instância sem
//! conectar aparelho e apaga no fim):
//!
//! ```text
//! POPH_EVOLUTION_REAL_URL=http://127.0.0.1:18081 \
//! POPH_EVOLUTION_REAL_CHAVE="$(grep '^EVO_APIKEY=' ~/poph-evolution/.env | cut -d= -f2-)" \
//!   cargo test -p poph-evolution --test evolucao_real -- --ignored --nocapture
//! ```

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use poph_evolution::{ConfigEvolution, EvolutionClient, EvolutionError};
use poph_types::Secret;

fn configurar(url: &str, chave: &str, instancia: &str) -> EvolutionClient {
    EvolutionClient::new(
        ConfigEvolution::new(url, instancia, Secret::new(chave.to_owned()))
            .expect("configuração")
            .with_timeout(Duration::from_secs(20)),
    )
    .expect("cliente")
}

#[tokio::test]
#[ignore = "precisa da Evolution isolada no ambiente (POPH_EVOLUTION_REAL_URL/_CHAVE)"]
async fn ciclo_de_instancia_sem_numero_contra_a_evolution_real() {
    let url = std::env::var("POPH_EVOLUTION_REAL_URL").expect("POPH_EVOLUTION_REAL_URL");
    let chave = std::env::var("POPH_EVOLUTION_REAL_CHAVE").expect("POPH_EVOLUTION_REAL_CHAVE");
    let agora = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("relógio")
        .as_secs();
    let instancia = format!("poph-teste-cliente-{agora}");
    let cliente = configurar(&url, &chave, &instancia);

    // Chave errada: 401/403 tipado, sem a chave na mensagem.
    let ruim = configurar(&url, "chave-errada-de-proposito", &instancia);
    let erro = ruim.list_instances().await.expect_err("chave errada");
    assert!(
        matches!(
            erro,
            EvolutionError::Response {
                status: 401 | 403,
                ..
            }
        ),
        "erro inesperado: {erro:?}"
    );
    assert!(!format!("{erro} {erro:?}").contains("chave-errada-de-proposito"));

    cliente.create_instance().await.expect("criar instância");
    // A verificação devolve Err em vez de dar panic: a instância tem de ser apagada mesmo assim.
    let verificacao: Result<(), String> = async {
        let pareamento = cliente
            .connect(None)
            .await
            .map_err(|e| format!("connect sem número: {e}"))?;
        if !pareamento
            .qr_base64
            .as_deref()
            .is_some_and(|qr| qr.starts_with("data:image"))
        {
            return Err("sem imagem do QR".to_owned());
        }
        if pareamento.pairing_code.is_some() {
            return Err("código de pareamento veio sem número".to_owned());
        }
        let estado = cliente
            .connection_state()
            .await
            .map_err(|e| format!("estado: {e}"))?;
        if estado == "open" {
            return Err("instância sem aparelho não pode estar aberta".to_owned());
        }
        let lista = cliente
            .list_instances()
            .await
            .map_err(|e| format!("listar: {e}"))?;
        if !lista.to_string().contains(&instancia) {
            return Err("instância não apareceu na lista".to_owned());
        }
        println!("estado={estado}");
        Ok(())
    }
    .await;
    let apagada = cliente.delete().await;
    verificacao.expect("verificação contra a Evolution real");
    apagada.expect("apagar instância de teste");
}
