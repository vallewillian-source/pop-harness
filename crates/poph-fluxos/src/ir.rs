//! A estrutura compilada de um fluxo: a IR `schema_versao = 1` de
//! `docs/FLUXOS_LINGUAGEM.md` §8.
//!
//! A IR é **estrutura, não texto**: nada de expressão em texto livre dentro dela (o texto do
//! script aparece só em [`Def::fonte`]). Ela é gravada em `fluxo_revisoes.definicao_json` na
//! publicação e não é recalculada em execução — o motor lê isto, e o compilador só recompila o
//! texto para conferir igualdade ([`crate::compilador::carregar`]).
//!
//! Duas regras de forma que a IR cumpre e os testes cobrem:
//!
//! - **Todo campo do esquema aparece.** O que o script não declarou entra com o padrão: lista
//!   vazia `[]`, `null` para texto opcional, `false` para booleano, `["agente"]` para
//!   `atores_acao`. Assim o JSON gravado é comparável byte a byte com a recompilação.
//! - **Ordem é a do script.** Etapas, decisões, fins e defs saem na ordem de declaração (fins: na
//!   ordem da primeira aparição), e nenhum deles é um mapa — a serialização sai sempre igual.
//!
//! Mudança futura só aditiva: campo novo com padrão; nenhum campo existente muda de significado;
//! campo desconhecido numa IR mais nova é ignorado na leitura, nunca erro.

use serde::{Deserialize, Serialize};

use poph_types::{ActionActor, FieldKind};

/// Versão do esquema que este crate produz e entende.
pub const SCHEMA_VERSAO: u32 = 1;

/// Estrutura compilada de um fluxo (o conteúdo de `fluxo_revisoes.definicao_json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ir {
    /// Versão do esquema; hoje sempre [`SCHEMA_VERSAO`].
    pub schema_versao: u32,
    /// Identificação do fluxo na organização.
    pub fluxo: Fluxo,
    /// O que o Jev lê para decidir iniciar (gate G9).
    pub gatilho: Gatilho,
    /// Id da etapa em que a execução começa.
    pub inicio: String,
    /// Etapas do atendimento, na ordem do script.
    pub etapas: Vec<Etapa>,
    /// Bifurcações, na ordem do script.
    pub decisoes: Vec<Decisao>,
    /// Desfechos, na ordem da primeira aparição no script.
    pub fins: Vec<Fim>,
    /// Funções de regra (`def`), na ordem do script.
    pub defs: Vec<Def>,
}

/// Cabeçalho do fluxo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fluxo {
    /// Id do fluxo na organização (o `slug`); não muda depois de publicado.
    pub id: String,
    /// Nome legível, mostrado ao atendente e no catálogo do G9.
    pub nome: String,
    /// Frase de catálogo; `null` quando o script não declarou.
    pub descricao: Option<String>,
}

/// Gatilho do gate G9, como o script o declara em `gatilho(...)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gatilho {
    /// Critério positivo (quando o fluxo deve começar).
    pub quando: String,
    /// Critério negativo; `null` quando não há.
    pub nao_quando: Option<String>,
    /// Falas típicas do cliente.
    pub exemplos: Vec<String>,
}

/// Uma etapa do atendimento.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Etapa {
    /// Id da etapa (segundo pedaço de `{etapa.campo}`).
    pub id: String,
    /// Nome curto que o agente e o atendente veem.
    pub nome: String,
    /// O que o agente deve fazer aqui; `null` quando vazio.
    pub instrucao: Option<String>,
    /// Quem pode executar ação nesta etapa.
    pub atores_acao: Vec<ActionActor>,
    /// Dados que a etapa coleta.
    pub campos: Vec<Campo>,
    /// Saídas da etapa.
    pub acoes: Vec<Acao>,
    /// Efeitos disparados ao entrar na etapa, antes de qualquer campo.
    pub ao_entrar: Vec<Efeito>,
}

/// Um dado que a etapa coleta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Campo {
    /// Id do campo dentro da etapa.
    pub id: String,
    /// Rótulo que o agente usa para perguntar.
    pub rotulo: String,
    /// Tipo do valor aceito.
    pub tipo: FieldKind,
    /// Se o campo é necessário para a etapa estar completa.
    pub obrigatorio: bool,
    /// Valores aceitos; só existe com `tipo = opcao`.
    pub opcoes: Vec<String>,
    /// Dica para quem edita o fluxo (a UI futura); o motor ignora.
    pub ajuda: Option<String>,
}

/// Uma saída da etapa.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Acao {
    /// Id da ação na etapa (é o nome que o agente chama).
    pub id: String,
    /// Descrição legível, mostrada na lista de ações do prompt.
    pub rotulo: String,
    /// Campos da própria etapa que precisam estar preenchidos.
    pub exige: Vec<String>,
    /// Restrição de quem pode executar; `null` = qualquer ator de `atores_acao`.
    pub ator: Option<ActionActor>,
    /// Para onde a execução vai quando a ação é executada.
    pub vai_para: Destino,
    /// Efeitos disparados junto da ação.
    pub efeitos: Vec<Efeito>,
}

/// Uma bifurcação: regra e dois destinos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decisao {
    /// Id da decisão (destino válido de `vai_para` e de outro ramo).
    pub id: String,
    /// A regra avaliada com os dados já registrados.
    pub se: Regra,
    /// Destino quando a regra é verdadeira.
    pub entao: Destino,
    /// Destino quando a regra é falsa.
    pub senao: Destino,
}

/// Um desfecho do fluxo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fim {
    /// Id do fim.
    pub id: String,
    /// Resumo gravado na execução; `null` quando o script não declarou.
    pub resultado: Option<String>,
    /// Se `true`, também encerra o atendimento (fecha conversa e Ficha).
    pub encerra_atendimento: bool,
}

/// Uma função de regra avançada (`def nome(dados)`), guardada como texto.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Def {
    /// Nome da função, usado por `funcao("nome")`.
    pub nome: String,
    /// O trecho exato do script, para o motor recompilar e conferir a IR.
    pub fonte: String,
}

/// Para onde uma ação ou um ramo leva.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Destino {
    /// Outra etapa.
    Etapa {
        /// Id da etapa de destino.
        id: String,
    },
    /// Uma decisão (avaliada na mesma passagem).
    Decisao {
        /// Id da decisão de destino.
        id: String,
    },
    /// Um desfecho: conclui a execução (e o atendimento, se pedido).
    Fim {
        /// Id do fim.
        id: String,
        /// Resumo gravado na execução.
        resultado: Option<String>,
        /// Se encerra o atendimento.
        encerra_atendimento: bool,
    },
}

impl Destino {
    /// Id do nó de destino (etapa, decisão ou fim).
    pub fn id(&self) -> &str {
        match self {
            Self::Etapa { id } | Self::Decisao { id } | Self::Fim { id, .. } => id,
        }
    }

    /// Id do nó quando ele continua o fluxo (etapa ou decisão); `None` no fim.
    ///
    /// É o que entra no grafo de alcance e de ciclo: o fim encerra a execução, então não há
    /// caminho que saia dele.
    pub fn aberto(&self) -> Option<&str> {
        match self {
            Self::Etapa { id } | Self::Decisao { id } => Some(id),
            Self::Fim { .. } => None,
        }
    }
}

/// Regra declarativa (árvore de combinadores), avaliada em Rust nativo pelo motor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Regra {
    /// Valor canônico do campo igual ao valor.
    Igual {
        /// `etapa.campo`.
        campo: String,
        /// Valor comparado.
        valor: String,
    },
    /// Valor canônico do campo diferente do valor.
    Diferente {
        /// `etapa.campo`.
        campo: String,
        /// Valor comparado.
        valor: String,
    },
    /// Valor numérico maior.
    Maior {
        /// `etapa.campo`.
        campo: String,
        /// Limite comparado.
        valor: String,
    },
    /// Valor numérico menor.
    Menor {
        /// `etapa.campo`.
        campo: String,
        /// Limite comparado.
        valor: String,
    },
    /// O campo contém o trecho (sem diferenciar maiúscula nem acento).
    Contem {
        /// `etapa.campo`.
        campo: String,
        /// Trecho procurado.
        valor: String,
    },
    /// O campo não foi preenchido.
    Vazio {
        /// `etapa.campo`.
        campo: String,
    },
    /// Todas as condições verdadeiras (2 ou mais).
    E {
        /// Condições.
        args: Vec<Regra>,
    },
    /// Qualquer condição verdadeira (2 ou mais).
    Ou {
        /// Condições.
        args: Vec<Regra>,
    },
    /// Inverte a condição (exatamente 1).
    Nao {
        /// Condição invertida.
        args: Vec<Regra>,
    },
    /// Chama a `def` de regra avançada.
    Funcao {
        /// Nome da `def`.
        nome: String,
    },
}

/// Efeito externo: falar com o cliente, avisar a equipe ou encaminhar o atendimento.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tipo", rename_all = "snake_case")]
pub enum Efeito {
    /// Texto fixo ao cliente, já interpolado.
    MensagemAoCliente {
        /// Texto enviado.
        texto: String,
    },
    /// Mensagem para outro número (equipe).
    NotificarWhatsapp {
        /// Número com DDI.
        para: String,
        /// Texto enviado.
        mensagem: String,
    },
    /// Transfere a conversa para um departamento da organização.
    Encaminhar {
        /// Nome do departamento na org.
        departamento: String,
    },
}

impl Ir {
    /// Id do fluxo na organização.
    pub fn fluxo_id(&self) -> &str {
        &self.fluxo.id
    }

    /// JSON compacto e estável: é a forma gravada e a forma comparada em `carregar`.
    pub fn para_json(&self) -> String {
        // Structs com `Vec` (nenhum mapa) serializam sempre na mesma ordem: determinístico.
        serde_json::to_string(self).expect("a IR é serializável por construção")
    }

    /// Lê a IR gravada.
    ///
    /// Campo desconhecido é ignorado (mudança aditiva); estrutura errada é erro de leitura, não
    /// pânico.
    pub fn de_json(texto: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(texto)
    }
}
