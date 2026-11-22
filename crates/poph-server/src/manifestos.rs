//! Catálogo de manifestos de plugin (HR-44c).
//!
//! O manifesto é a única fonte do formulário de configuração e da lista de segredos de uma
//! integração — e ele só existe no handshake do plugin (`poph_types::Manifesto`, protocolo em
//! `docs/PLUGIN_PROTOCOLO.md`). Este módulo resolve a pergunta "qual é o manifesto do `kind` X"
//! com duas decisões:
//!
//! - **de onde vem:** do programa declarado em `[plugins].integracoes` na configuração do
//!   servidor. Nada vem da requisição: o cliente diz o `kind`, o servidor já sabe qual binário
//!   executa.
//! - **como não subir o plugin a cada requisição:** o primeiro pedido sobe o plugin pelo
//!   [`Supervisor`] (o mesmo da HR-25/27, com handshake e prazo), lê o manifesto, encerra o
//!   processo e guarda o resultado em memória. As requisições seguintes leem o cache. Falha também
//!   fica em cache por [`VALIDADE_DA_FALHA`], para um binário quebrado não virar laço de processos.
//!   O cache dura a vida do processo: o manifesto muda quando o plugin muda, e aí o servidor
//!   reinicia.
//!
//! [`CatalogoManifestos::listar`] (HR-48b) responde "quais tipos existem": percorre os declarados e
//! usa exatamente a mesma resolução, para a tela não precisar adivinhar o `kind`.
//!
//! Todo erro aqui é seguro para o cliente: o motivo detalhado (caminho do programa, erro do
//! sistema) vai para o log, e a resposta leva só o `kind` e uma frase de operação.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use poph_plugins::{ColetorEventos, OpcoesPlugin, Supervisor};
use poph_types::{IntegrationId, Manifesto, OrgId, PluginDeclarado, PluginsConfig};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;

/// Quanto tempo uma falha de handshake fica em cache antes de tentar de novo.
pub const VALIDADE_DA_FALHA: Duration = Duration::from_secs(30);

/// O que deu errado ao resolver o manifesto de um `kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FalhaManifesto {
    /// O `kind` não está em `[plugins].integracoes`: não há programa para perguntar.
    NaoDeclarado,
    /// O plugin foi declarado mas não respondeu ao handshake (programa ausente, prazo, protocolo).
    Indisponivel { motivo: String },
}

impl FalhaManifesto {
    /// Mensagem para o operador — sem caminho de arquivo e sem detalhe interno.
    pub fn mensagem(&self) -> String {
        match self {
            Self::NaoDeclarado => "esta instalação não declara manifesto para este tipo de \
                                   integração; declare o plugin em `[plugins].integracoes` da \
                                   configuração"
                .to_owned(),
            Self::Indisponivel { .. } => {
                "o plugin deste tipo não respondeu ao handshake; verifique \
                                          o programa declarado em `[plugins].integracoes`"
                    .to_owned()
            }
        }
    }
}

/// Situação de um plugin declarado na lista de manifestos (HR-48b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SituacaoManifesto {
    /// O plugin respondeu ao handshake e o manifesto está disponível.
    Ok,
    /// O plugin foi declarado mas não respondeu; a lista continua, este item fica marcado.
    Indisponivel,
}

/// Um item da lista de plugins declarados (HR-48b).
///
/// Só o que é do plugin: o `kind` da configuração e o que o handshake devolveu (nome e versão).
/// **Nunca** caminho do programa, argumentos, configuração de integração ou segredo — a lista é
/// metadado da instalação, não do cliente.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResumoManifesto {
    /// Tipo da integração (`kind`) declarado em `[plugins].integracoes`.
    pub kind: String,
    /// Se o manifesto pôde ser lido agora.
    pub situacao: SituacaoManifesto,
    /// Nome legível do plugin, quando o manifesto foi lido.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nome: Option<String>,
    /// Versão do plugin, quando o manifesto foi lido.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub versao: Option<String>,
    /// Frase de operação, só quando o manifesto não pôde ser lido (sem caminho nem detalhe).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mensagem: Option<String>,
}

/// Resultado guardado em cache (sucesso ou falha, com o instante em que foi obtido).
struct Entrada {
    resultado: Result<Manifesto, String>,
    quando: Instant,
}

/// Manifestos dos plugins declarados, resolvidos sob demanda e guardados em memória.
pub struct CatalogoManifestos {
    declarados: BTreeMap<String, PluginDeclarado>,
    cache: Mutex<HashMap<String, Entrada>>,
}

impl std::fmt::Debug for CatalogoManifestos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogoManifestos")
            .field("declarados", &self.declarados.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl CatalogoManifestos {
    /// Catálogo a partir da configuração (nada é executado aqui).
    pub fn novo(plugins: &PluginsConfig) -> Self {
        let declarados = plugins
            .integracoes
            .iter()
            .map(|plugin| (plugin.kind.trim().to_owned(), plugin.clone()))
            .collect();
        Self {
            declarados,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Tipos com plugin declarado (a tela usa para saber quais `kind` têm manifesto).
    pub fn declarados(&self) -> Vec<&str> {
        self.declarados.keys().map(String::as_str).collect()
    }

    /// Lista dos plugins declarados, na ordem do `kind`, resolvendo o manifesto de cada um.
    ///
    /// É a rota da HR-48b: a tela precisa saber **quais** tipos existem antes de consultar um por
    /// um. Um plugin que não responde **não derruba a lista** — o item sai com
    /// [`SituacaoManifesto::Indisponivel`] e uma frase de operação; o motivo detalhado (caminho do
    /// programa, erro do sistema) fica só no log, como na consulta por tipo.
    ///
    /// A resolução reaproveita o cache: a primeira listagem sobe cada plugin uma vez; as seguintes
    /// leem da memória, inclusive as falhas (por [`VALIDADE_DA_FALHA`]).
    pub async fn listar(&self) -> Vec<ResumoManifesto> {
        let mut itens = Vec::with_capacity(self.declarados.len());
        for kind in self.declarados.keys() {
            let item = match self.manifesto(kind).await {
                Ok(manifesto) => ResumoManifesto {
                    kind: kind.clone(),
                    situacao: SituacaoManifesto::Ok,
                    nome: Some(manifesto.nome),
                    versao: Some(manifesto.versao),
                    mensagem: None,
                },
                Err(falha) => ResumoManifesto {
                    kind: kind.clone(),
                    situacao: SituacaoManifesto::Indisponivel,
                    nome: None,
                    versao: None,
                    mensagem: Some(falha.mensagem()),
                },
            };
            itens.push(item);
        }
        itens
    }

    /// Manifesto do `kind`, do cache ou do handshake. Nunca sobe dois processos do mesmo `kind`
    /// ao mesmo tempo: o `Mutex` serializa as resoluções.
    pub async fn manifesto(&self, kind: &str) -> Result<Manifesto, FalhaManifesto> {
        let declarado = self
            .declarados
            .get(kind)
            .cloned()
            .ok_or(FalhaManifesto::NaoDeclarado)?;
        let mut cache = self.cache.lock().await;
        if let Some(entrada) = cache.get(kind) {
            let expirada =
                entrada.resultado.is_err() && entrada.quando.elapsed() >= VALIDADE_DA_FALHA;
            if !expirada {
                return entrada
                    .resultado
                    .clone()
                    .map_err(|motivo| FalhaManifesto::Indisponivel { motivo });
            }
        }
        let resultado = resolver(&declarado).await;
        let guardado = resultado.clone().map_err(|falha| match falha {
            FalhaManifesto::Indisponivel { motivo } => motivo,
            FalhaManifesto::NaoDeclarado => "não declarado".to_owned(),
        });
        cache.insert(
            kind.to_owned(),
            Entrada {
                resultado: guardado,
                quando: Instant::now(),
            },
        );
        resultado
    }
}

/// Sobe o plugin, lê o manifesto e encerra o processo.
async fn resolver(declarado: &PluginDeclarado) -> Result<Manifesto, FalhaManifesto> {
    let mut opcoes = OpcoesPlugin::novo(
        declarado.programa.clone(),
        IntegrationId::generate(),
        OrgId::generate(),
    )
    .com_kind(declarado.kind.trim());
    opcoes.argumentos = declarado.argumentos.clone();
    // O manifesto não depende de configuração nem de segredo: nada do operador é enviado aqui.
    match Supervisor::iniciar(opcoes, Arc::new(ColetorEventos::novo())).await {
        Ok(supervisor) => {
            let manifesto = supervisor.manifesto();
            supervisor.encerrar().await;
            match manifesto {
                Some(manifesto) => Ok(manifesto),
                None => {
                    tracing::warn!(
                        kind = %declarado.kind,
                        "o plugin declarou handshake sem manifesto"
                    );
                    Err(FalhaManifesto::Indisponivel {
                        motivo: "handshake sem manifesto".to_owned(),
                    })
                }
            }
        }
        Err(erro) => {
            // Caminho do programa e detalhe do sistema ficam só no log.
            tracing::warn!(
                kind = %declarado.kind,
                programa = %declarado.programa.display(),
                erro = %erro,
                "não consegui ler o manifesto do plugin"
            );
            Err(FalhaManifesto::Indisponivel {
                motivo: erro.to_string(),
            })
        }
    }
}

// ───────────────────────────── Validação de configuração ─────────────────────────────

/// Um campo recusado pela validação contra o esquema do manifesto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampoInvalido {
    /// Nome do campo no esquema (ex.: `api_key`).
    pub campo: String,
    /// Por que foi recusado, em português, para mostrar ao operador.
    pub motivo: String,
}

/// Valida um objeto contra o subconjunto de JSON Schema que o manifesto usa.
///
/// Cobre o que o Console desenha e o que o plugin consome: `type` (`object`, `string`, `number`,
/// `integer`, `boolean`, `array`), `required`, `enum`, `minLength`/`maxLength`,
/// `minimum`/`maximum` e `properties` aninhados. **Campo fora do esquema é recusado** (ao contrário
/// do padrão do JSON Schema): erro de digitação não pode virar configuração silenciosamente
/// ignorada pelo plugin. `additionalProperties: true` libera, para quem precisar.
pub fn validar_objeto(valor: &Value, esquema: &Value, prefixo: &str) -> Result<(), CampoInvalido> {
    let objeto = valor.as_object().ok_or_else(|| CampoInvalido {
        campo: campo(prefixo, ""),
        motivo: "precisa ser um objeto".to_owned(),
    })?;
    let vazio = serde_json::Map::new();
    let propriedades = esquema
        .get("properties")
        .and_then(Value::as_object)
        .unwrap_or(&vazio);
    if let Some(obrigatorios) = esquema.get("required").and_then(Value::as_array) {
        for nome in obrigatorios.iter().filter_map(Value::as_str) {
            let presente = objeto
                .get(nome)
                .is_some_and(|v| !v.is_null() && v.as_str() != Some(""));
            if !presente {
                return Err(CampoInvalido {
                    campo: campo(prefixo, nome),
                    motivo: "obrigatório".to_owned(),
                });
            }
        }
    }
    let adicional_liberado = match esquema.get("additionalProperties") {
        Some(Value::Bool(permitido)) => *permitido,
        Some(Value::Object(_)) => true,
        _ => false,
    };
    for (nome, valor_do_campo) in objeto {
        let Some(esquema_do_campo) = propriedades.get(nome) else {
            if adicional_liberado {
                continue;
            }
            return Err(CampoInvalido {
                campo: campo(prefixo, nome),
                motivo: "não está no esquema do plugin".to_owned(),
            });
        };
        validar_campo(nome, valor_do_campo, esquema_do_campo, prefixo)?;
    }
    Ok(())
}

/// Valida um valor contra o esquema do próprio campo.
fn validar_campo(
    nome: &str,
    valor: &Value,
    esquema: &Value,
    prefixo: &str,
) -> Result<(), CampoInvalido> {
    let erro = |motivo: String| CampoInvalido {
        campo: campo(prefixo, nome),
        motivo,
    };
    if let Some(tipo) = esquema.get("type").and_then(Value::as_str) {
        let confere = match tipo {
            "string" => valor.is_string(),
            "number" => valor.is_number(),
            "integer" => valor.as_i64().is_some() || valor.as_u64().is_some(),
            "boolean" => valor.is_boolean(),
            "array" => valor.is_array(),
            "object" => valor.is_object(),
            "null" => valor.is_null(),
            outro => {
                return Err(erro(format!(
                    "o esquema do plugin usa o tipo `{outro}`, que a API não valida"
                )));
            }
        };
        if !confere {
            return Err(erro(format!("precisa ser do tipo `{tipo}`")));
        }
    }
    if let Some(permitidos) = esquema.get("enum").and_then(Value::as_array)
        && !permitidos.iter().any(|permitido| permitido == valor)
    {
        let lista: Vec<String> = permitidos
            .iter()
            .map(|permitido| permitido.to_string())
            .collect();
        return Err(erro(format!(
            "valor fora da lista aceita: {}",
            lista.join(", ")
        )));
    }
    if let (Some(texto), Some(minimo)) = (
        valor.as_str(),
        esquema.get("minLength").and_then(Value::as_u64),
    ) && (texto.chars().count() as u64) < minimo
    {
        return Err(erro(format!("precisa de pelo menos {minimo} caractere(s)")));
    }
    if let (Some(texto), Some(maximo)) = (
        valor.as_str(),
        esquema.get("maxLength").and_then(Value::as_u64),
    ) && (texto.chars().count() as u64) > maximo
    {
        return Err(erro(format!("aceita no máximo {maximo} caractere(s)")));
    }
    if let Some(numero) = valor.as_f64() {
        if let Some(minimo) = esquema.get("minimum").and_then(Value::as_f64)
            && numero < minimo
        {
            return Err(erro(format!("precisa ser maior ou igual a {minimo}")));
        }
        if let Some(maximo) = esquema.get("maximum").and_then(Value::as_f64)
            && numero > maximo
        {
            return Err(erro(format!("precisa ser menor ou igual a {maximo}")));
        }
    }
    if valor.is_object() && esquema.get("properties").is_some() {
        validar_objeto(valor, esquema, &campo(prefixo, nome))?;
    }
    Ok(())
}

/// Caminho do campo na resposta de erro (`config_json.token` / `secrets.token`).
fn campo(prefixo: &str, nome: &str) -> String {
    if prefixo.is_empty() {
        nome.to_owned()
    } else if nome.is_empty() {
        prefixo.to_owned()
    } else {
        format!("{prefixo}.{nome}")
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use serde_json::json;

    fn esquema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "modo": { "type": "string", "enum": ["inverter"] },
                "remetente": { "type": "string", "minLength": 3 },
                "tentativas": { "type": "integer", "minimum": 1, "maximum": 5 },
                "avisar": { "type": "boolean" },
                "rotulos": { "type": "array" }
            },
            "required": ["remetente"]
        })
    }

    fn erro(valor: Value) -> CampoInvalido {
        validar_objeto(&valor, &esquema(), "config_json").expect_err("deveria recusar")
    }

    #[test]
    fn aceita_objeto_que_segue_o_esquema() {
        validar_objeto(
            &json!({"modo": "inverter", "remetente": "Loja", "tentativas": 2, "avisar": false, "rotulos": []}),
            &esquema(),
            "config_json",
        )
        .expect("válido");
    }

    #[test]
    fn campo_obrigatorio_ausente_ou_vazio_e_recusado() {
        assert_eq!(erro(json!({})).campo, "config_json.remetente");
        assert_eq!(erro(json!({})).motivo, "obrigatório");
        assert_eq!(
            erro(json!({"remetente": "  "})).campo,
            "config_json.remetente"
        );
    }

    #[test]
    fn tipo_errado_e_recusado_no_campo() {
        let falha = erro(json!({"remetente": "Loja", "tentativas": "duas"}));
        assert_eq!(falha.campo, "config_json.tentativas");
        assert!(falha.motivo.contains("integer"), "{}", falha.motivo);
    }

    #[test]
    fn valor_fora_do_enum_e_recusado_no_campo() {
        let falha = erro(json!({"remetente": "Loja", "modo": "outro"}));
        assert_eq!(falha.campo, "config_json.modo");
        assert!(falha.motivo.contains("inverter"), "{}", falha.motivo);
    }

    #[test]
    fn faixa_de_numero_e_comprimento_sao_conferidos() {
        assert_eq!(
            erro(json!({"remetente": "Loja", "tentativas": 9})).campo,
            "config_json.tentativas"
        );
        assert_eq!(
            erro(json!({"remetente": "Lo"})).campo,
            "config_json.remetente"
        );
    }

    #[test]
    fn campo_fora_do_esquema_e_recusado_salvo_se_liberado() {
        assert_eq!(
            erro(json!({"remetente": "Loja", "inventado": 1})).campo,
            "config_json.inventado"
        );
        let mut liberado = esquema();
        liberado["additionalProperties"] = json!(true);
        validar_objeto(
            &json!({"remetente": "Loja", "extra": 1}),
            &liberado,
            "config_json",
        )
        .expect("additionalProperties libera");
    }

    #[test]
    fn erro_no_campo_aninhado_traz_o_caminho() {
        let esquema = json!({
            "type": "object",
            "properties": {
                "canal": {
                    "type": "object",
                    "properties": { "nome": { "type": "string" } },
                    "required": ["nome"]
                }
            }
        });
        let falha = validar_objeto(&json!({"canal": {}}), &esquema, "config_json")
            .expect_err("faltou nome");
        assert_eq!(falha.campo, "config_json.canal.nome");
    }

    #[test]
    fn valor_que_nao_e_objeto_e_recusado_no_prefixo() {
        let falha = validar_objeto(&json!(["a"]), &esquema(), "secrets").expect_err("lista");
        assert_eq!(falha.campo, "secrets");
    }
}
