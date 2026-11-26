//! Ferramentas internas do turno (PRD §6, passo 6, com a decisão D-04).
//!
//! O motor traz **o laço** de ferramentas, o limite de rodadas e o filtro da lista habilitada por
//! agente. As ferramentas de negócio entram por [`RegistroPadrao`]: `consultar_base` na HR-17,
//! `buscar_produto` e o carrinho na HR-18/HR-19, `enviar_arquivo` na HR-24 e `transferir` com o Jev
//! (HR-29) — cada uma registra a sua sem mexer no laço.
//!
//! Regras que já valem aqui:
//! - **D-04:** só é oferecida ao modelo a ferramenta que está na lista habilitada do agente *e* que
//!   o registro sabe executar. Ferramenta desabilitada não aparece no pedido.
//! - Chamada de ferramenta que o modelo inventou (ou desabilitada) vira **resultado de erro**, não
//!   execução: o modelo é informado e pode seguir sem ela.
//! - A ferramenta recebe o contexto do turno por parâmetro; não acessa estado global.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use poph_core::llm::Tool;
use poph_types::{AgentId, ConversationId, OrgId, ProductId, TurnId};
use serde::Serialize;

/// Futuro de execução de uma ferramenta.
pub type FuturoFerramenta = Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'static>>;

/// O que a ferramenta sabe do turno em que foi chamada.
#[derive(Debug, Clone)]
pub struct ContextoFerramenta {
    /// Org dona do turno.
    pub org: OrgId,
    /// Conversa do turno.
    pub conversa: ConversationId,
    /// Agente que está atendendo.
    pub agente: AgentId,
    /// Turno em andamento (para gravar efeitos ligados a ele).
    pub turno: TurnId,
    /// Coletor do que as ferramentas usaram neste turno (vai para `turns.retrieved_json`).
    pub uso: Arc<UsoFerramentas>,
}

/// Produto que a ferramenta usou numa resposta, com o preço **do registro** no momento da consulta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProdutoUsado {
    /// Id do produto.
    pub produto: ProductId,
    /// Nome.
    pub nome: String,
    /// Preço em centavos aplicado.
    pub preco_centavos: i64,
    /// Preço como o cliente vê.
    pub preco_texto: String,
    /// Unidade ("por visita").
    pub unidade: String,
    /// Condições.
    pub condicoes: String,
}

/// O que as ferramentas de um turno usaram. O turno cria uma instância, entrega no
/// [`ContextoFerramenta`] e grava o resultado em `turns.retrieved_json` no fim.
///
/// A coleta é por produto e sem repetição: o mesmo produto citado em duas chamadas aparece uma vez,
/// com o preço da primeira menção (que é o preço que o modelo viu).
#[derive(Debug, Default)]
pub struct UsoFerramentas {
    produtos: Mutex<Vec<ProdutoUsado>>,
}

impl UsoFerramentas {
    /// Coletor vazio.
    pub fn nova() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Registra um produto usado (ignora repetição pelo id).
    pub fn registrar_produto(&self, produto: ProdutoUsado) {
        let mut lista = self
            .produtos
            .lock()
            .unwrap_or_else(|erro| erro.into_inner());
        if lista.iter().any(|atual| atual.produto == produto.produto) {
            return;
        }
        lista.push(produto);
    }

    /// Produtos usados, na ordem em que apareceram.
    pub fn produtos(&self) -> Vec<ProdutoUsado> {
        self.produtos
            .lock()
            .unwrap_or_else(|erro| erro.into_inner())
            .clone()
    }

    /// `true` quando nada foi usado (o registro do turno pode ficar `[]`).
    pub fn vazio(&self) -> bool {
        self.produtos
            .lock()
            .unwrap_or_else(|erro| erro.into_inner())
            .is_empty()
    }
}

/// Teto de caracteres de cada resumo de chamada gravado no turno (argumentos e resultado).
pub const TETO_RESUMO_FERRAMENTA: usize = 1024;

/// Marca que fecha um resumo cortado pelo [`TETO_RESUMO_FERRAMENTA`].
pub const MARCA_DE_CORTE: &str = "…[cortado]";

/// Motivo de recusa: a ferramenta não está na lista habilitada do agente (D-04).
pub const MOTIVO_NAO_HABILITADA: &str = "nao_habilitada";
/// Motivo de recusa: o nome não existe no registro deste processo.
pub const MOTIVO_DESCONHECIDA: &str = "desconhecida";
/// Motivo de recusa: os argumentos não batem com o contrato da ferramenta.
pub const MOTIVO_ARGUMENTO_INVALIDO: &str = "argumento_invalido";
/// Motivo de recusa: a ferramenta falhou ao executar (banco, busca, configuração).
pub const MOTIVO_ERRO_DE_EXECUCAO: &str = "erro_de_execucao";
/// Motivo de recusa: passou do teto de chamadas por mensagem do agente.
pub const MOTIVO_LIMITE_DE_CHAMADAS: &str = "limite_de_chamadas";
/// Motivo de recusa: o turno tirou a ferramenta de escopo (G3 em `nada`, catálogo já consultado).
pub const MOTIVO_FORA_DO_TURNO: &str = "fora_do_turno";

/// Como a chamada terminou, para o registro do turno.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desfecho {
    /// A ferramenta rodou (mesmo quando devolveu `Err`, que é recusa de conteúdo ao modelo).
    Executada,
    /// A chamada foi recusada antes de rodar, por este motivo.
    Recusada(&'static str),
}

/// Uma chamada de ferramenta como ela entra no registro do turno (`turns.tools_json`).
///
/// `argumentos` e `resultado` são **resumos de texto** (o JSON compacto dos argumentos e o texto
/// devolvido ao modelo), cortados em [`TETO_RESUMO_FERRAMENTA`]. Argumentos com nome de campo
/// sensível têm o valor trocado por `[redigido]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChamadaRegistrada {
    /// Nome que o modelo pediu.
    pub nome: String,
    /// Rodada do laço de ferramentas (0 = primeira chamada ao modelo).
    pub rodada: u32,
    /// Ordem da chamada dentro do turno, começando em 1.
    pub ordem: u32,
    /// `executada` ou `recusada`.
    pub estado: &'static str,
    /// Motivo da recusa; `None` quando a ferramenta rodou.
    pub motivo: Option<&'static str>,
    /// Tempo gasto na execução (0 nas recusas antes de rodar).
    pub duracao_ms: u64,
    /// Resumo dos argumentos (JSON compacto, redigido e cortado).
    pub argumentos: String,
    /// Resumo do que voltou para o modelo (texto cortado).
    pub resultado: String,
}

/// Coletor das chamadas de ferramenta de um turno (HR-47b).
///
/// O turno cria uma instância, entrega em [`ContextoFerramenta`] e grava o resultado em
/// `turns.tools_json` no fim, junto com os nomes (`nomes`), que é o contrato antigo.
#[derive(Debug, Default)]
pub struct RegistroChamadas {
    chamadas: Mutex<Vec<ChamadaRegistrada>>,
}

impl RegistroChamadas {
    /// Coletor vazio.
    pub fn nova() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Registra uma chamada. `ordem` é atribuída pela ordem de chegada, começando em 1.
    pub fn registrar(
        &self,
        nome: &str,
        rodada: u32,
        desfecho: Desfecho,
        duracao: Duration,
        argumentos: &serde_json::Value,
        resultado: &str,
    ) {
        let mut lista = self
            .chamadas
            .lock()
            .unwrap_or_else(|erro| erro.into_inner());
        let ordem = u32::try_from(lista.len())
            .unwrap_or(u32::MAX)
            .saturating_add(1);
        let (estado, motivo) = match desfecho {
            Desfecho::Executada => ("executada", None),
            Desfecho::Recusada(motivo) => ("recusada", Some(motivo)),
        };
        lista.push(ChamadaRegistrada {
            nome: nome.to_owned(),
            rodada,
            ordem,
            estado,
            motivo,
            duracao_ms: u64::try_from(duracao.as_millis()).unwrap_or(u64::MAX),
            argumentos: resumo_de_argumentos(argumentos),
            resultado: cortar(resultado),
        });
    }

    /// Chamadas registradas, na ordem em que aconteceram.
    pub fn chamadas(&self) -> Vec<ChamadaRegistrada> {
        self.chamadas
            .lock()
            .unwrap_or_else(|erro| erro.into_inner())
            .clone()
    }

    /// Chamadas como lista JSON (o que vai no registro do turno).
    pub fn para_json(&self) -> serde_json::Value {
        serde_json::to_value(self.chamadas())
            .unwrap_or_else(|_| serde_json::Value::Array(Vec::new()))
    }

    /// `true` quando nenhuma chamada foi registrada.
    pub fn vazio(&self) -> bool {
        self.chamadas
            .lock()
            .unwrap_or_else(|erro| erro.into_inner())
            .is_empty()
    }
}

/// JSON que vai para `turns.tools_json`.
///
/// Mantém `nomes` no formato que a leitura já publica (`tools`) e acrescenta `chamadas`, o detalhe
/// por chamada (HR-47b). Turno antigo gravou só a lista: a leitura aceita as duas formas.
pub fn registro_do_turno(nomes: &[String], chamadas: &RegistroChamadas) -> String {
    serde_json::json!({
        "nomes": nomes,
        "chamadas": chamadas.para_json(),
    })
    .to_string()
}

/// Resumo dos argumentos: JSON compacto, com valor trocado por `[redigido]` em campo sensível, e
/// cortado no teto.
pub fn resumo_de_argumentos(argumentos: &serde_json::Value) -> String {
    cortar(&redigir(argumentos).to_string())
}

/// Troca o valor de qualquer campo cujo nome pareça segredo. É defesa simples e deliberada: o
/// argumento é texto do modelo, e o registro não pode virar cópia de credencial.
fn redigir(valor: &serde_json::Value) -> serde_json::Value {
    match valor {
        serde_json::Value::Object(mapa) => serde_json::Value::Object(
            mapa.iter()
                .map(|(chave, valor)| {
                    let valor = if chave_sensivel(chave) {
                        serde_json::Value::String("[redigido]".to_owned())
                    } else {
                        redigir(valor)
                    };
                    (chave.clone(), valor)
                })
                .collect(),
        ),
        serde_json::Value::Array(itens) => {
            serde_json::Value::Array(itens.iter().map(redigir).collect())
        }
        outro => outro.clone(),
    }
}

/// Nome de campo que indica segredo (comparação sem caixa e por trecho).
fn chave_sensivel(chave: &str) -> bool {
    const SENSIVEIS: &[&str] = &[
        "senha",
        "token",
        "secret",
        "segredo",
        "chave",
        "password",
        "api_key",
        "apikey",
        "authorization",
        "credencial",
    ];
    let minuscula = chave.to_lowercase();
    SENSIVEIS.iter().any(|palavra| minuscula.contains(palavra))
}

/// Corta o texto no teto, em limite de caractere, marcando o corte.
pub fn cortar(texto: &str) -> String {
    if texto.chars().count() <= TETO_RESUMO_FERRAMENTA {
        return texto.to_owned();
    }
    let mut saida: String = texto.chars().take(TETO_RESUMO_FERRAMENTA).collect();
    saida.push_str(MARCA_DE_CORTE);
    saida
}

/// Uma ferramenta interna que o modelo pode chamar.
pub trait FerramentaInterna: Send + Sync {
    /// Nome exato que o modelo escreve na chamada (um de `FERRAMENTAS_V1`).
    fn nome(&self) -> &'static str;
    /// Texto que explica ao modelo quando usar.
    fn descricao(&self) -> &'static str;
    /// JSON Schema dos argumentos (objeto).
    fn parametros(&self) -> serde_json::Value;
    /// Executa a chamada. `Err` é o texto de erro devolvido ao modelo, não uma falha do turno.
    fn executar(
        &self,
        contexto: ContextoFerramenta,
        argumentos: serde_json::Value,
    ) -> FuturoFerramenta;
}

/// Quem sabe dizer qual ferramenta existe neste processo.
///
/// A HR-17/HR-18/HR-19/HR-24 registram as suas aqui; até lá, o registro de produção é
/// [`SemFerramentas`] e o modelo recebe o pedido sem ferramenta nenhuma.
pub trait RegistroFerramentas: Send + Sync {
    /// A ferramenta com esse nome, quando implementada neste processo.
    fn ferramenta(&self, nome: &str) -> Option<Arc<dyn FerramentaInterna>>;
}

/// Registro vazio da V1 até a fase 2. Mantém o laço de ferramentas testável e o toolset honesto.
#[derive(Debug, Default, Clone, Copy)]
pub struct SemFerramentas;

impl RegistroFerramentas for SemFerramentas {
    fn ferramenta(&self, _nome: &str) -> Option<Arc<dyn FerramentaInterna>> {
        None
    }
}

/// Registro com as ferramentas de negócio do processo (HR-17 em diante).
///
/// A HR-17 traz `consultar_base`; produtos, arquivos e transferência entram nas tarefas delas pelo
/// mesmo canal, sem tocar no laço do turno.
#[derive(Default, Clone)]
pub struct RegistroPadrao {
    ferramentas: HashMap<String, Arc<dyn FerramentaInterna>>,
}

impl RegistroPadrao {
    /// Registro vazio.
    pub fn novo() -> Self {
        Self::default()
    }

    /// Registra (ou substitui) uma ferramenta pelo nome dela.
    pub fn registrar(&mut self, ferramenta: Arc<dyn FerramentaInterna>) {
        self.ferramentas
            .insert(ferramenta.nome().to_owned(), ferramenta);
    }

    /// Versão encadeável de [`RegistroPadrao::registrar`].
    pub fn com(mut self, ferramenta: Arc<dyn FerramentaInterna>) -> Self {
        self.registrar(ferramenta);
        self
    }

    /// Nomes registrados, em ordem alfabética (diagnóstico e testes).
    pub fn nomes(&self) -> Vec<String> {
        let mut nomes: Vec<String> = self.ferramentas.keys().cloned().collect();
        nomes.sort();
        nomes
    }
}

impl RegistroFerramentas for RegistroPadrao {
    fn ferramenta(&self, nome: &str) -> Option<Arc<dyn FerramentaInterna>> {
        self.ferramentas.get(nome).cloned()
    }
}

/// Lista que vai no pedido ao modelo: as habilitadas do agente (D-04), na ordem delas, restritas às
/// que o registro sabe executar.
pub fn oferecidas(registro: &dyn RegistroFerramentas, habilitadas: &[String]) -> Vec<Tool> {
    habilitadas
        .iter()
        .filter_map(|nome| registro.ferramenta(nome))
        .map(|ferramenta| Tool {
            nome: ferramenta.nome().to_owned(),
            descricao: ferramenta.descricao().to_owned(),
            parametros: ferramenta.parametros(),
        })
        .collect()
}

#[cfg(test)]
mod testes {
    use super::*;

    struct Sempre;

    impl FerramentaInterna for Sempre {
        fn nome(&self) -> &'static str {
            "consultar_base"
        }

        fn descricao(&self) -> &'static str {
            "consulta a base"
        }

        fn parametros(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        fn executar(
            &self,
            _contexto: ContextoFerramenta,
            _argumentos: serde_json::Value,
        ) -> FuturoFerramenta {
            Box::pin(async { Ok("ok".to_owned()) })
        }
    }

    struct UmRegistro;

    impl RegistroFerramentas for UmRegistro {
        fn ferramenta(&self, nome: &str) -> Option<Arc<dyn FerramentaInterna>> {
            (nome == "consultar_base").then(|| Arc::new(Sempre) as Arc<dyn FerramentaInterna>)
        }
    }

    #[test]
    fn sem_registro_nao_oferece_nada_mesmo_habilitado() {
        let lista = oferecidas(&SemFerramentas, &["consultar_base".to_owned()]);
        assert!(lista.is_empty());
    }

    #[test]
    fn oferece_so_o_que_esta_habilitado_e_existe() {
        let registro = UmRegistro;
        let lista = oferecidas(
            &registro,
            &[
                "consultar_base".to_owned(),
                "buscar_produto".to_owned(),
                "transferir".to_owned(),
            ],
        );
        assert_eq!(lista.len(), 1);
        assert_eq!(lista[0].nome, "consultar_base");
    }

    #[test]
    fn registro_guarda_execucao_com_ordem_e_rodada() {
        let registro = RegistroChamadas::nova();
        registro.registrar(
            "consultar_base",
            0,
            Desfecho::Executada,
            Duration::from_millis(7),
            &serde_json::json!({"consulta": "prazo"}),
            "Prazo de 5 dias úteis.",
        );
        let chamadas = registro.chamadas();
        assert_eq!(chamadas.len(), 1);
        assert_eq!(chamadas[0].nome, "consultar_base");
        assert_eq!(chamadas[0].rodada, 0);
        assert_eq!(chamadas[0].ordem, 1);
        assert_eq!(chamadas[0].estado, "executada");
        assert_eq!(chamadas[0].motivo, None);
        assert_eq!(chamadas[0].duracao_ms, 7);
        assert!(chamadas[0].argumentos.contains("prazo"));
        assert_eq!(chamadas[0].resultado, "Prazo de 5 dias úteis.");
    }

    #[test]
    fn registro_guarda_recusa_com_motivo() {
        let registro = RegistroChamadas::nova();
        registro.registrar(
            "cart_add_item",
            1,
            Desfecho::Recusada(MOTIVO_NAO_HABILITADA),
            Duration::ZERO,
            &serde_json::Value::Null,
            "erro: a ferramenta `cart_add_item` não está habilitada para este agente",
        );
        let chamadas = registro.chamadas();
        assert_eq!(chamadas[0].estado, "recusada");
        assert_eq!(chamadas[0].motivo, Some(MOTIVO_NAO_HABILITADA));
        assert_eq!(chamadas[0].duracao_ms, 0);
    }

    #[test]
    fn argumento_com_nome_de_segredo_e_redigido() {
        let resumo = resumo_de_argumentos(&serde_json::json!({
            "termo": "baratas",
            "token": "segredo-abc",
            "api_key": "chave-xyz",
            "password": "senha-123",
            "aninhado": {"chave_de_acesso": "outro-segredo", "ok": "visivel"},
        }));
        assert!(resumo.contains("baratas"));
        assert!(resumo.contains("visivel"));
        assert!(resumo.contains("[redigido]"));
        for proibido in ["segredo-abc", "chave-xyz", "senha-123", "outro-segredo"] {
            assert!(!resumo.contains(proibido), "vazou `{proibido}`: {resumo}");
        }
    }

    #[test]
    fn resumo_e_cortado_no_teto_com_marca() {
        let grande = "a".repeat(TETO_RESUMO_FERRAMENTA * 3);
        let resumo = cortar(&grande);
        assert_eq!(
            resumo.chars().count(),
            TETO_RESUMO_FERRAMENTA + MARCA_DE_CORTE.chars().count()
        );
        assert!(resumo.ends_with(MARCA_DE_CORTE));
        assert_eq!(cortar("curto"), "curto");
    }

    #[test]
    fn registro_do_turno_traz_nomes_e_chamadas() {
        let registro = RegistroChamadas::nova();
        registro.registrar(
            "consultar_base",
            0,
            Desfecho::Executada,
            Duration::ZERO,
            &serde_json::json!({}),
            "ok",
        );
        let json: serde_json::Value = serde_json::from_str(&registro_do_turno(
            &["consultar_base".to_owned()],
            &registro,
        ))
        .expect("JSON do registro");
        assert_eq!(json["nomes"], serde_json::json!(["consultar_base"]));
        assert_eq!(json["chamadas"].as_array().expect("lista").len(), 1);
    }
}
