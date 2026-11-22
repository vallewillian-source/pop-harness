//! Tipos do módulo de Fluxos e Fichas: situações, atores, tipos de campo, eventos e o gatilho
//! que o gate G9 lê.
//!
//! Espelho da especificação (`docs/FLUXOS_LINGUAGEM.md` §8, IR `schema_versao = 1`) e das colunas
//! do PRD `HARNESS_FLUXOS_PRD.md` §7.1. Aqui só existe **forma**: o texto que vai para o banco e
//! para a API, a ida e volta entre os dois e a validação de forma do gatilho. Quem decide transição,
//! permissão e efeito são o motor (`poph-fluxos`) e o armazenamento (`poph-store`).
//!
//! Duas regras de contrato que valem em todo o módulo:
//!
//! - **Um texto por valor.** Cada enum tem uma lista fechada de textos (`TODOS`), e o mesmo texto é
//!   usado na coluna, no JSON, no `Display` e no `FromStr`. Valor fora da lista é recusado com
//!   [`DomainError::Validacao`], com a lista de aceitos na mensagem — nunca vira um valor "solto".
//! - **Dois atores, nunca um.** [`AtorAcao`] responde "quem pode executar esta ação na etapa"
//!   (`etapas[].atores_acao` e `acoes[].ator` da IR); [`AtorEvento`] responde "quem assinou este
//!   evento na Ficha" (`ficha_eventos.ator_tipo`). O cliente não é ator de nenhum dos dois (decisão
//!   D5 do mantenedor).

use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{DomainError, DomainResult};
use crate::ids::FluxoId;

/// Define um enum de lista fechada com um texto único por variante.
///
/// Gera a enum, [`TODOS`](StatusFicha::TODOS), `como_texto`, `de_texto`, `Display`, `FromStr`,
/// `Serialize` e `Deserialize`. A vantagem de gerar tudo do mesmo lugar é que não existe uma
/// segunda lista para divergir: mudar o texto de uma variante muda banco, JSON e API de uma vez.
macro_rules! define_enum_texto {
    (
        $(#[$meta:meta])*
        $nome:ident, campo: $campo:literal,
        $(
            $(#[$vmeta:meta])*
            $variante:ident => $texto:literal
        ),+ $(,)?
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $nome {
            $(
                $(#[$vmeta])*
                $variante,
            )+
        }

        impl $nome {
            /// Todos os valores da lista fechada, na ordem de declaração.
            pub const TODOS: &'static [Self] = &[$(Self::$variante),+];

            /// Texto gravado na coluna e trafegado no JSON.
            pub fn como_texto(self) -> &'static str {
                match self {
                    $(Self::$variante => $texto,)+
                }
            }

            /// Lê o texto da coluna ou do JSON. Valor fora da lista é recusado com a lista de
            /// aceitos na mensagem.
            pub fn de_texto(valor: &str) -> DomainResult<Self> {
                match valor {
                    $($texto => Ok(Self::$variante),)+
                    outro => Err(DomainError::Validacao {
                        campo: $campo,
                        motivo: format!(
                            "valor desconhecido `{outro}`; aceitos: {}",
                            Self::TODOS
                                .iter()
                                .map(|variante| variante.como_texto())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    }),
                }
            }
        }

        impl fmt::Display for $nome {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.como_texto())
            }
        }

        impl FromStr for $nome {
            type Err = DomainError;

            fn from_str(texto: &str) -> Result<Self, Self::Err> {
                Self::de_texto(texto)
            }
        }

        impl Serialize for $nome {
            fn serialize<S: Serializer>(&self, serializador: S) -> Result<S::Ok, S::Error> {
                serializador.serialize_str(self.como_texto())
            }
        }

        impl<'de> Deserialize<'de> for $nome {
            fn deserialize<D: Deserializer<'de>>(deserializador: D) -> Result<Self, D::Error> {
                let texto = String::deserialize(deserializador)?;
                Self::de_texto(&texto).map_err(D::Error::custom)
            }
        }
    };
}

define_enum_texto!(
    /// Situação da Ficha (`fichas.status`). Uma Ficha nasce aberta com a conversa e fecha quando o
    /// atendimento fecha; mensagem depois do fechamento abre outra Ficha (PRD L5).
    StatusFicha, campo: "status_ficha",
    /// O atendimento está correndo: aceita evento, execução e ação.
    Aberta => "open",
    /// O atendimento acabou: não aceita nada novo.
    Fechada => "closed",
);

define_enum_texto!(
    /// Quem encerrou a Ficha (`fichas.fechada_por`). `migracao` só aparece no backfill das Fichas
    /// que já existiam quando o módulo entrou.
    FechadaPor, campo: "fechada_por",
    /// O próprio cliente pediu para encerrar.
    Cliente => "cliente",
    /// O agente encerrou (ferramenta `atendimento_encerrar`).
    Agente => "agente",
    /// Um atendente humano encerrou pela API.
    Atendente => "atendente",
    /// Um token de administração encerrou pela API.
    Admin => "admin",
    /// O fim de um fluxo com `encerra_atendimento = True` encerrou.
    Fluxo => "fluxo",
    /// O sistema encerrou (retenção, exclusão de fluxo, rotina).
    Sistema => "sistema",
    /// Ficha criada pelo backfill da migração, já fechada com a conversa.
    Migracao => "migracao",
);

define_enum_texto!(
    /// Situação de uma execução de fluxo (`fluxo_execucoes.status`).
    ///
    /// Só existe **uma** `ativa` por Ficha (PRD L6): é o índice único parcial do banco que garante,
    /// não este tipo.
    StatusExecucao, campo: "status_execucao",
    /// Em andamento: é a execução que o prompt e as ferramentas usam.
    Ativa => "ativa",
    /// Chegou a um `fim(...)`, com resultado.
    Concluida => "concluida",
    /// Interrompida de fora: atendimento encerrado, fluxo excluído, abandonada pelo atendente.
    Abandonada => "abandonada",
    /// Parou por erro de execução (regra que falhou, ciclo de decisão, IR divergente).
    Falha => "falha",
);

define_enum_texto!(
    /// Situação de uma etapa dentro da execução (`fluxo_execucao_etapas.status`).
    StatusEtapa, campo: "status_etapa",
    /// Entrou e ainda não preencheu nenhum campo.
    Pendente => "pendente",
    /// Preencheu parte dos campos obrigatórios.
    Parcial => "parcial",
    /// Campos obrigatórios completos: a etapa já pode ser concluída por uma ação.
    Concluida => "concluida",
);

define_enum_texto!(
    /// Quem pode **executar uma ação** na etapa (`etapas[].atores_acao` e `acoes[].ator` da IR).
    ///
    /// `None` (ausente no JSON) significa "qualquer ator da etapa", no campo `acoes[].ator`.
    AtorAcao, campo: "ator_acao",
    /// O agente de IA (ferramenta do turno).
    Agente => "agente",
    /// O atendente humano (API, Console; depois, o módulo Matrix).
    Atendente => "atendente",
);

define_enum_texto!(
    /// Quem **assinou o evento** na Ficha (`ficha_eventos.ator_tipo`).
    ///
    /// É mais largo que [`AtorAcao`] porque a Ficha registra também o que o sistema e o Jev fazem.
    AtorEvento, campo: "ator_evento",
    /// O agente de IA.
    Agente => "agente",
    /// O atendente humano.
    Atendente => "atendente",
    /// O próprio Harness (início, mudança de etapa, efeito, fechamento).
    Sistema => "sistema",
    /// O Jev (decisão do G9 que iniciou ou não o fluxo).
    Jev => "jev",
    /// Um token de administração.
    Admin => "admin",
);

define_enum_texto!(
    /// Tipo do evento da Ficha (`ficha_eventos.tipo`) — a linha do tempo que o atendente lê.
    ///
    /// Lista fechada de propósito: cada tipo novo é uma decisão de contrato (o Console e o módulo
    /// Matrix casam pelo texto).
    TipoEventoFicha, campo: "tipo_evento_ficha",
    /// A Ficha nasceu (junto com a conversa).
    FichaAberta => "ficha_aberta",
    /// Uma execução de fluxo começou (com a decisão do Jev quando foi o G9).
    FluxoIniciado => "fluxo_iniciado",
    /// O motor entrou numa etapa.
    EtapaEntrou => "etapa_entrou",
    /// Campos de uma etapa foram gravados.
    CamposPreenchidos => "campos_preenchidos",
    /// Uma ação da etapa foi executada (e para onde levou).
    AcaoExecutada => "acao_executada",
    /// Uma decisão foi resolvida (ramo escolhido e destino).
    DecisaoResolvida => "decisao_resolvida",
    /// Uma etapa foi concluída.
    EtapaConcluida => "etapa_concluida",
    /// Um efeito externo entrou na fila durável.
    EfeitoCriado => "efeito_criado",
    /// O efeito foi entregue.
    EfeitoEnviado => "efeito_enviado",
    /// O efeito falhou (a transição continua valendo, PRD L11).
    EfeitoFalhou => "efeito_falhou",
    /// A execução chegou a um fim.
    ExecucaoConcluida => "execucao_concluida",
    /// A execução foi abandonada (atendimento encerrado, fluxo excluído, ação do atendente).
    ExecucaoAbandonada => "execucao_abandonada",
    /// A execução parou por erro.
    ExecucaoFalhou => "execucao_falhou",
    /// A conversa foi encaminhada para um departamento.
    AtendimentoTransferido => "atendimento_transferido",
    /// A Ficha (e a conversa) foi encerrada.
    FichaFechada => "ficha_fechada",
);

define_enum_texto!(
    /// Situação de um efeito na fila durável (`fluxo_efeitos.estado`).
    EstadoEfeito, campo: "estado_efeito",
    /// Na fila, ainda não tentado.
    Pendente => "pendente",
    /// Em entrega neste momento (o trabalhador pegou).
    Enviando => "enviando",
    /// Entregue.
    Enviado => "enviado",
    /// Passou do limite de tentativas (ou erro definitivo).
    Falhou => "falhou",
);

define_enum_texto!(
    /// Tipo de um campo de etapa (`campos[].tipo` da IR).
    ///
    /// O valor canônico de cada tipo (o que as regras comparam) está em `FLUXOS_LINGUAGEM.md` §5.
    TipoCampo, campo: "tipo_campo",
    /// Texto livre do cliente (dado, nunca instrução).
    Texto => "texto",
    /// Número (`maior`/`menor` só valem aqui).
    Numero => "numero",
    /// Data ISO 8601 (`AAAA-MM-DD` ou com hora e fuso).
    Data => "data",
    /// `true`/`false` (canônico `sim`/`nao`).
    Booleano => "booleano",
    /// Um dos `opcoes` declarados no campo.
    Opcao => "opcao",
    /// Anexo: guarda nome, tipo e tamanho; o conteúdo não entra no fluxo.
    Anexo => "anexo",
);

/// Gatilho de início de um fluxo, como o gate G9 (FX-22) o lê e como o script o declara
/// (`gatilho(...)`, `FLUXOS_LINGUAGEM.md` §4.2).
///
/// O gate **só escolhe**: quem valida (fluxo ativo, liberado ao agente, revisão publicada) e inicia
/// é o código do turno. Os textos vão para o Jev cortados no teto de 400 caracteres, e o corte é de
/// quem monta a pergunta, não deste tipo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatilhoFluxo {
    /// Fluxo dono do gatilho (id estável, o que a execução grava).
    pub id: FluxoId,
    /// Slug do fluxo na org (`fluxo.id` do script): é o rótulo que o Jev devolve na escolha.
    pub slug: String,
    /// Critério positivo: quando este fluxo deve começar.
    pub quando: String,
    /// Critério negativo: quando **não** deve começar (ausente = não há ressalva).
    #[serde(default)]
    pub nao_quando: Option<String>,
    /// Falas típicas do cliente, como exemplos para o Jev.
    #[serde(default)]
    pub exemplos: Vec<String>,
}

impl GatilhoFluxo {
    /// Gatilho com o obrigatório preenchido (negativo e exemplos ficam vazios).
    pub fn novo(id: FluxoId, slug: impl Into<String>, quando: impl Into<String>) -> Self {
        Self {
            id,
            slug: slug.into(),
            quando: quando.into(),
            nao_quando: None,
            exemplos: Vec::new(),
        }
    }

    /// Gatilho com o critério negativo.
    pub fn com_nao_quando(mut self, nao_quando: impl Into<String>) -> Self {
        self.nao_quando = Some(nao_quando.into());
        self
    }

    /// Gatilho com os exemplos, na ordem dada.
    pub fn com_exemplos<I, S>(mut self, exemplos: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.exemplos = exemplos.into_iter().map(Into::into).collect();
        self
    }

    /// Valida a **forma** (não a força do critério): slug e critério positivo não podem ser vazios,
    /// e exemplo em branco é ruído no Jev — recusado em vez de enviado.
    pub fn validar(&self) -> DomainResult<()> {
        if self.slug.trim().is_empty() {
            return Err(DomainError::Validacao {
                campo: "gatilho.slug",
                motivo: "o fluxo precisa de um slug para o Jev escolher".to_owned(),
            });
        }
        if self.quando.trim().is_empty() {
            return Err(DomainError::Validacao {
                campo: "gatilho.quando",
                motivo: "sem o critério positivo o Jev não tem como escolher este fluxo".to_owned(),
            });
        }
        if self
            .exemplos
            .iter()
            .any(|exemplo| exemplo.trim().is_empty())
        {
            return Err(DomainError::Validacao {
                campo: "gatilho.exemplos",
                motivo: "exemplo em branco não ajuda o Jev: remova ou escreva a fala".to_owned(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde::de::IntoDeserializer;
    use serde::de::value::{Error as ValueError, StrDeserializer};
    use uuid::Uuid;

    use super::*;
    use crate::ids::{EfeitoId, EventoFichaId, ExecucaoId, FichaId, RevisaoId};

    /// UUID versão 4 fixo (a feature `v4` não está ligada no workspace).
    const UUID_V4: &str = "f47ac10b-58cc-4372-a567-0e02b2c3d479";

    /// Ida e volta texto ⇄ enum, para cada valor da lista e para o `Display`.
    macro_rules! confere_ida_e_volta {
        ($nome:ident) => {{
            assert!(!$nome::TODOS.is_empty(), "lista fechada vazia");
            for variante in $nome::TODOS {
                let texto = variante.como_texto();
                let lido = $nome::de_texto(texto)
                    .unwrap_or_else(|erro| panic!("`{texto}` recusado: {erro}"));
                assert_eq!(
                    lido, *variante,
                    "texto `{texto}` voltou como outra variante"
                );
                assert_eq!(variante.to_string(), texto, "Display diverge de como_texto");
                assert_eq!(
                    texto
                        .parse::<$nome>()
                        .expect("FromStr aceita o texto canônico"),
                    *variante
                );
            }
            // Não existem dois valores com o mesmo texto (o JSON ficaria ambíguo).
            let mut textos: Vec<&str> = $nome::TODOS.iter().map(|v| v.como_texto()).collect();
            textos.sort_unstable();
            let total = textos.len();
            textos.dedup();
            assert_eq!(
                textos.len(),
                total,
                "texto repetido na lista de {}",
                stringify!($nome)
            );
        }};
    }

    #[test]
    fn cada_enum_faz_ida_e_volta_em_texto() {
        confere_ida_e_volta!(StatusFicha);
        confere_ida_e_volta!(FechadaPor);
        confere_ida_e_volta!(StatusExecucao);
        confere_ida_e_volta!(StatusEtapa);
        confere_ida_e_volta!(AtorAcao);
        confere_ida_e_volta!(AtorEvento);
        confere_ida_e_volta!(TipoEventoFicha);
        confere_ida_e_volta!(EstadoEfeito);
        confere_ida_e_volta!(TipoCampo);
    }

    /// Valor fora da lista é recusado, e a mensagem diz o campo e o que é aceito.
    macro_rules! confere_recusa {
        ($nome:ident, $campo:literal) => {{
            let erro = $nome::de_texto("valor_que_nao_existe")
                .expect_err("valor desconhecido deve ser recusado");
            match &erro {
                DomainError::Validacao { campo, motivo } => {
                    assert_eq!(campo, &$campo, "campo do erro");
                    assert!(
                        motivo.contains("valor_que_nao_existe"),
                        "motivo sem o valor: {motivo}"
                    );
                    assert!(
                        motivo.contains($nome::TODOS[0].como_texto()),
                        "motivo sem a lista de aceitos: {motivo}"
                    );
                }
                outro => panic!("erro inesperado: {outro:?}"),
            }
            assert!(
                "valor_que_nao_existe".parse::<$nome>().is_err(),
                "FromStr deve recusar como de_texto"
            );
        }};
    }

    #[test]
    fn cada_enum_recusa_valor_desconhecido_com_a_lista_de_aceitos() {
        confere_recusa!(StatusFicha, "status_ficha");
        confere_recusa!(FechadaPor, "fechada_por");
        confere_recusa!(StatusExecucao, "status_execucao");
        confere_recusa!(StatusEtapa, "status_etapa");
        confere_recusa!(AtorAcao, "ator_acao");
        confere_recusa!(AtorEvento, "ator_evento");
        confere_recusa!(TipoEventoFicha, "tipo_evento_ficha");
        confere_recusa!(EstadoEfeito, "estado_efeito");
        confere_recusa!(TipoCampo, "tipo_campo");
    }

    /// O JSON usa o mesmo texto da coluna, e valor desconhecido não desserializa.
    macro_rules! confere_json {
        ($nome:ident, $texto:literal, $variante:ident) => {{
            let json = serde_json::to_value($nome::$variante).expect("serializa");
            assert_eq!(json, serde_json::json!($texto));

            let lido: $nome = serde_json::from_value(json).expect("desserializa");
            assert_eq!(lido, $nome::$variante);

            let errado: Result<$nome, _> = serde_json::from_value(serde_json::json!("outro"));
            assert!(errado.is_err(), "JSON com valor fora da lista deve falhar");
        }};
    }

    #[test]
    fn json_usa_o_mesmo_texto_da_coluna() {
        confere_json!(StatusFicha, "open", Aberta);
        confere_json!(StatusFicha, "closed", Fechada);
        confere_json!(StatusExecucao, "ativa", Ativa);
        confere_json!(StatusEtapa, "concluida", Concluida);
        confere_json!(FechadaPor, "migracao", Migracao);
        confere_json!(AtorAcao, "atendente", Atendente);
        confere_json!(AtorEvento, "jev", Jev);
        confere_json!(TipoEventoFicha, "decisao_resolvida", DecisaoResolvida);
        confere_json!(EstadoEfeito, "enviando", Enviando);
        confere_json!(TipoCampo, "booleano", Booleano);
    }

    #[test]
    fn deserialize_recusa_texto_desconhecido() {
        // Passa pela mesma validação do `de_texto`, não pelo `Deserialize` derivado.
        let entrada: StrDeserializer<'_, ValueError> = "aberta".into_deserializer();
        let erro = StatusFicha::deserialize(entrada).expect_err("`aberta` é rótulo, não texto");
        assert!(
            erro.to_string().contains("valor desconhecido"),
            "erro: {erro}"
        );
    }

    #[test]
    fn atores_nao_aceitam_cliente() {
        // Decisão D5: o cliente nunca é ator; provedor e operador do POP Atende também não existem.
        for texto in ["cliente", "client", "provider", "operator", "system"] {
            assert!(
                AtorAcao::de_texto(texto).is_err(),
                "AtorAcao aceitou `{texto}`"
            );
            assert!(
                AtorEvento::de_texto(texto).is_err(),
                "AtorEvento aceitou `{texto}`"
            );
        }
    }

    #[test]
    fn ids_do_modulo_fazem_ida_e_volta_e_recusam_versao_errada() {
        macro_rules! confere_id {
            ($nome:ident) => {{
                let id = $nome::generate();
                let texto = id.to_string();
                assert_eq!(texto.parse::<$nome>().expect("ida e volta"), id);
                assert_eq!(texto.len(), 36, "forma textual de UUID");

                let erro = UUID_V4
                    .parse::<$nome>()
                    .expect_err("UUID v4 deve ser recusado");
                assert!(
                    matches!(erro, DomainError::IdInvalido { .. }),
                    "erro inesperado: {erro:?}"
                );
            }};
        }

        confere_id!(FichaId);
        confere_id!(FluxoId);
        confere_id!(RevisaoId);
        confere_id!(ExecucaoId);
        confere_id!(EventoFichaId);
        confere_id!(EfeitoId);

        // Cada id é o seu próprio tipo: o UUID de uma entidade não vira id de outra.
        let uuid = Uuid::now_v7();
        assert!(FichaId::from_uuid(uuid).is_ok());
        assert_eq!(
            FluxoId::from_uuid(uuid)
                .expect("mesmo UUID, tipo certo")
                .as_uuid(),
            uuid
        );
    }

    #[test]
    fn gatilho_faz_ida_e_volta_em_json() {
        let gatilho = GatilhoFluxo::novo(
            FluxoId::generate(),
            "agendar_visita",
            "cliente quer marcar visita",
        )
        .com_nao_quando("cliente já tem visita marcada")
        .com_exemplos(["queria marcar uma visita", "tem horário amanhã?"]);
        gatilho.validar().expect("gatilho bem formado");

        let json = serde_json::to_value(&gatilho).expect("serializa");
        assert_eq!(json["slug"], serde_json::json!("agendar_visita"));
        assert_eq!(
            json["nao_quando"],
            serde_json::json!("cliente já tem visita marcada")
        );
        let lido: GatilhoFluxo = serde_json::from_value(json).expect("desserializa");
        assert_eq!(lido, gatilho);

        // Campo opcional ausente entra com o padrão, para a IR antiga continuar legível.
        let minimo = serde_json::json!({
            "id": gatilho.id,
            "slug": "agendar_visita",
            "quando": "cliente quer marcar visita"
        });
        let lido: GatilhoFluxo = serde_json::from_value(minimo).expect("desserializa mínimo");
        assert_eq!(lido.nao_quando, None);
        assert!(lido.exemplos.is_empty());
    }

    #[test]
    fn gatilho_recusa_forma_vazia() {
        let id = FluxoId::generate();

        let sem_slug = GatilhoFluxo::novo(id, "  ", "cliente quer marcar visita");
        assert!(matches!(
            sem_slug.validar(),
            Err(DomainError::Validacao {
                campo: "gatilho.slug",
                ..
            })
        ));

        let sem_criterio = GatilhoFluxo::novo(id, "agendar_visita", "\t");
        assert!(matches!(
            sem_criterio.validar(),
            Err(DomainError::Validacao {
                campo: "gatilho.quando",
                ..
            })
        ));

        let com_exemplo_vazio =
            GatilhoFluxo::novo(id, "agendar_visita", "cliente quer marcar visita")
                .com_exemplos(["queria marcar uma visita", " "]);
        assert!(matches!(
            com_exemplo_vazio.validar(),
            Err(DomainError::Validacao {
                campo: "gatilho.exemplos",
                ..
            })
        ));
    }
}
