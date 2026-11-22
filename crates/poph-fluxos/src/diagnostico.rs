//! Diagnósticos de forma do compilador de fluxos (FX-10).
//!
//! A lista é **fechada** e é a de `docs/FLUXOS_LINGUAGEM.md` §10: 26 erros (`FLX01`–`FLX28`) e 15
//! avisos (`FLX51`–`FLX65`). Código novo entra naquele documento antes de entrar aqui.
//!
//! O diagnóstico aponta **onde** o autor tem de mexer, de duas formas possíveis:
//!
//! - **Linha e coluna** (1-based) do texto do script: é o diagnóstico do compilador, que lê a
//!   árvore sintática (`FLX01`–`FLX28`, `FLX51`–`FLX64`).
//! - **Caminho do nó na IR** (`etapas[2] (id "entrega") > acoes[1] > vai_para`): é o diagnóstico de
//!   revalidação de uma estrutura que **não** veio do compilador (importação, revisão antiga); ali
//!   não existe texto para apontar. Ver [`crate::validador`].
//!
//! O `FLX65` (departamento que a organização não tem) só nasce sobre a IR, na publicação.
//!
//! Erro bloqueia a publicação; aviso não. As falhas de **execução** (`FLXR01`–`FLXR06`) não são
//! diagnóstico de compilação e não estão nesta enum: elas nascem no motor (FX-11 em diante).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Código do diagnóstico, na ordem da lista fechada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Codigo {
    /// Sintaxe inválida (o parser aponta linha e coluna).
    Sintaxe,
    /// Fonte acima do limite de bytes.
    FonteGrande,
    /// Construção fora do dialeto (`load`, `while`, `lambda`, `class`, `try`, `import`, f-string).
    Dialeto,
    /// Limite estourado na carga (ticks, heap, profundidade, prazo).
    LimiteDeCarga,
    /// Item no topo que não é declaração.
    TopoInvalido,
    /// Valor com tipo errado (ou que não é literal) num argumento.
    TipoErrado,
    /// Nome que não existe na linguagem de fluxos.
    NomeInexistente,
    /// Falta `fluxo(...)` ou há mais de um.
    FluxoAusente,
    /// Argumento obrigatório ausente.
    ArgumentoAusente,
    /// Argumento desconhecido.
    ArgumentoDesconhecido,
    /// Id fora do formato.
    IdInvalido,
    /// Id repetido.
    IdRepetido,
    /// Destino inexistente.
    DestinoInexistente,
    /// Mesmo `fim` com argumentos diferentes.
    FimDivergente,
    /// `exige` cita campo que não existe na etapa.
    ExigeInexistente,
    /// Problema nas `opcoes` do campo.
    OpcoesInvalidas,
    /// Regra ou ator inválido.
    RegraOuAtorInvalido,
    /// Tipo de campo desconhecido.
    TipoDesconhecido,
    /// Combinador malformado.
    CombinadorMalformado,
    /// Combinador cita etapa ou campo inexistente.
    CampoInexistente,
    /// Ciclo entre decisões.
    CicloDeDecisao,
    /// `def` fora do formato.
    DefInvalida,
    /// Variável com formato inválido.
    VariavelInvalida,
    /// `{...}` em texto que não interpola.
    InterpolacaoProibida,
    /// Acima do teto de nós.
    NosDemais,
    /// `notificar_whatsapp` com número inválido.
    NumeroInvalido,
    /// Etapa sem nenhuma ação.
    EtapaSemAcao,
    /// Etapa ou decisão inalcançável.
    Inalcancavel,
    /// Campo que ninguém usa.
    CampoNaoUsado,
    /// Comparação com valor fora das opções.
    ForaDasOpcoes,
    /// Condição que sempre dá o mesmo resultado.
    CondicaoContraditoria,
    /// Gatilho sem `exemplos`.
    GatilhoSemExemplos,
    /// Gatilho sem `nao_quando`.
    GatilhoSemNegativo,
    /// `notificar_whatsapp` sem variável.
    NotificacaoSemVariavel,
    /// Variável que aponta para etapa/campo inexistente.
    VariavelSemCampo,
    /// `maior`/`menor` em campo que não é `numero`.
    ComparacaoDeTexto,
    /// `def` definida e não usada.
    DefNaoUsada,
    /// Ator da etapa sem nenhuma ação dele.
    AtorSemAcao,
    /// `ao_entrar` usa variável do próprio passo.
    VariavelDoProprioPasso,
    /// Texto acima do recomendado.
    TextoLongo,
    /// Departamento citado em `encaminhar` que a organização não tem (ou tem com nome repetido).
    DepartamentoInexistente,
}

impl Codigo {
    /// Os 41 códigos, na ordem de `FLUXOS_LINGUAGEM.md` §10.
    pub const TODOS: &'static [Self] = &[
        Self::Sintaxe,
        Self::FonteGrande,
        Self::Dialeto,
        Self::LimiteDeCarga,
        Self::TopoInvalido,
        Self::TipoErrado,
        Self::NomeInexistente,
        Self::FluxoAusente,
        Self::ArgumentoAusente,
        Self::ArgumentoDesconhecido,
        Self::IdInvalido,
        Self::IdRepetido,
        Self::DestinoInexistente,
        Self::FimDivergente,
        Self::ExigeInexistente,
        Self::OpcoesInvalidas,
        Self::RegraOuAtorInvalido,
        Self::TipoDesconhecido,
        Self::CombinadorMalformado,
        Self::CampoInexistente,
        Self::CicloDeDecisao,
        Self::DefInvalida,
        Self::VariavelInvalida,
        Self::InterpolacaoProibida,
        Self::NosDemais,
        Self::NumeroInvalido,
        Self::EtapaSemAcao,
        Self::Inalcancavel,
        Self::CampoNaoUsado,
        Self::ForaDasOpcoes,
        Self::CondicaoContraditoria,
        Self::GatilhoSemExemplos,
        Self::GatilhoSemNegativo,
        Self::NotificacaoSemVariavel,
        Self::VariavelSemCampo,
        Self::ComparacaoDeTexto,
        Self::DefNaoUsada,
        Self::AtorSemAcao,
        Self::VariavelDoProprioPasso,
        Self::TextoLongo,
        Self::DepartamentoInexistente,
    ];

    /// Os códigos que o **compilador** emite, lendo o texto do script: todos os de [`Self::TODOS`]
    /// menos o `FLX65`, que só nasce sobre uma IR já compilada (`validador::resolver_departamentos`).
    pub fn do_compilador() -> impl Iterator<Item = Self> {
        Self::TODOS
            .iter()
            .copied()
            .filter(|codigo| *codigo != Self::DepartamentoInexistente)
    }

    /// Texto do código (`FLX12`), que é o que aparece na API e nos testes.
    pub fn como_texto(self) -> &'static str {
        match self {
            Self::Sintaxe => "FLX01",
            Self::FonteGrande => "FLX02",
            Self::Dialeto => "FLX03",
            Self::LimiteDeCarga => "FLX04",
            Self::TopoInvalido => "FLX05",
            Self::TipoErrado => "FLX06",
            Self::NomeInexistente => "FLX07",
            Self::FluxoAusente => "FLX10",
            Self::ArgumentoAusente => "FLX11",
            Self::ArgumentoDesconhecido => "FLX12",
            Self::IdInvalido => "FLX13",
            Self::IdRepetido => "FLX14",
            Self::DestinoInexistente => "FLX15",
            Self::FimDivergente => "FLX16",
            Self::ExigeInexistente => "FLX17",
            Self::OpcoesInvalidas => "FLX18",
            Self::RegraOuAtorInvalido => "FLX19",
            Self::TipoDesconhecido => "FLX20",
            Self::CombinadorMalformado => "FLX21",
            Self::CampoInexistente => "FLX22",
            Self::CicloDeDecisao => "FLX23",
            Self::DefInvalida => "FLX24",
            Self::VariavelInvalida => "FLX25",
            Self::InterpolacaoProibida => "FLX26",
            Self::NosDemais => "FLX27",
            Self::NumeroInvalido => "FLX28",
            Self::EtapaSemAcao => "FLX51",
            Self::Inalcancavel => "FLX52",
            Self::CampoNaoUsado => "FLX53",
            Self::ForaDasOpcoes => "FLX54",
            Self::CondicaoContraditoria => "FLX55",
            Self::GatilhoSemExemplos => "FLX56",
            Self::GatilhoSemNegativo => "FLX57",
            Self::NotificacaoSemVariavel => "FLX58",
            Self::VariavelSemCampo => "FLX59",
            Self::ComparacaoDeTexto => "FLX60",
            Self::DefNaoUsada => "FLX61",
            Self::AtorSemAcao => "FLX62",
            Self::VariavelDoProprioPasso => "FLX63",
            Self::TextoLongo => "FLX64",
            Self::DepartamentoInexistente => "FLX65",
        }
    }

    /// Código a partir do texto (`FLX12`), para ler de log, teste ou API.
    pub fn de_texto(texto: &str) -> Option<Self> {
        Self::TODOS
            .iter()
            .copied()
            .find(|c| c.como_texto() == texto)
    }
}

impl fmt::Display for Codigo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.como_texto())
    }
}

impl FromStr for Codigo {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::de_texto(s).ok_or(())
    }
}

impl Serialize for Codigo {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.como_texto())
    }
}

impl<'de> Deserialize<'de> for Codigo {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let texto = String::deserialize(d)?;
        Self::de_texto(&texto).ok_or_else(|| {
            serde::de::Error::custom(format!("código `{texto}` não existe na lista fechada"))
        })
    }
}

/// Gravidade do diagnóstico: erro bloqueia a publicação, aviso não.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severidade {
    /// Bloqueia a publicação.
    Erro,
    /// Não bloqueia: é dica para o autor.
    Aviso,
}

impl Severidade {
    /// Texto usado no JSON e na API.
    pub fn como_texto(self) -> &'static str {
        match self {
            Self::Erro => "erro",
            Self::Aviso => "aviso",
        }
    }
}

impl fmt::Display for Severidade {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.como_texto())
    }
}

/// Um diagnóstico, com o ponto exato de onde o autor tem de mexer.
///
/// O ponto é **um dos dois**, nunca os dois: linha e coluna do script (diagnóstico do compilador,
/// 1-based, como o autor conta e como a API publica — diagnóstico sem trecho, como o limite de
/// tamanho da fonte, aponta `1:1`) ou o caminho do nó na IR (revalidação de estrutura que não veio
/// do compilador). No JSON o que não existe não aparece.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostico {
    /// Código da lista fechada.
    pub codigo: Codigo,
    /// Erro ou aviso.
    pub severidade: Severidade,
    /// Mensagem em português, dizendo o que corrigir.
    pub mensagem: String,
    /// Linha do trecho no script (1-based); ausente quando o diagnóstico veio de uma IR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linha: Option<u32>,
    /// Coluna do trecho no script (1-based); ausente quando o diagnóstico veio de uma IR.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coluna: Option<u32>,
    /// Caminho do nó na IR (`etapas[2] (id "entrega") > acoes[1] > vai_para`); ausente quando o
    /// diagnóstico aponta linha e coluna.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caminho: Option<String>,
}

impl Diagnostico {
    /// Diagnóstico de erro no ponto dado do texto do script.
    pub fn erro(codigo: Codigo, mensagem: impl Into<String>, linha: u32, coluna: u32) -> Self {
        Self {
            codigo,
            severidade: Severidade::Erro,
            mensagem: mensagem.into(),
            linha: Some(linha),
            coluna: Some(coluna),
            caminho: None,
        }
    }

    /// Diagnóstico de aviso no ponto dado do texto do script.
    pub fn aviso(codigo: Codigo, mensagem: impl Into<String>, linha: u32, coluna: u32) -> Self {
        Self {
            codigo,
            severidade: Severidade::Aviso,
            mensagem: mensagem.into(),
            linha: Some(linha),
            coluna: Some(coluna),
            caminho: None,
        }
    }

    /// Diagnóstico de erro que aponta o **nó da IR** (`etapas[2] (id "entrega") > acoes[1]`).
    ///
    /// É o formato da revalidação de uma estrutura que não veio do compilador: ali não há texto
    /// para dar linha e coluna.
    pub fn erro_no(
        codigo: Codigo,
        mensagem: impl Into<String>,
        caminho: impl Into<String>,
    ) -> Self {
        Self {
            codigo,
            severidade: Severidade::Erro,
            mensagem: mensagem.into(),
            linha: None,
            coluna: None,
            caminho: Some(caminho.into()),
        }
    }

    /// Diagnóstico de aviso que aponta o nó da IR.
    pub fn aviso_no(
        codigo: Codigo,
        mensagem: impl Into<String>,
        caminho: impl Into<String>,
    ) -> Self {
        Self {
            codigo,
            severidade: Severidade::Aviso,
            mensagem: mensagem.into(),
            linha: None,
            coluna: None,
            caminho: Some(caminho.into()),
        }
    }

    /// Linha do trecho no script, quando o diagnóstico aponta texto.
    pub fn linha(&self) -> Option<u32> {
        self.linha
    }

    /// Coluna do trecho no script, quando o diagnóstico aponta texto.
    pub fn coluna(&self) -> Option<u32> {
        self.coluna
    }

    /// Caminho do nó na IR, quando o diagnóstico aponta estrutura.
    pub fn caminho(&self) -> Option<&str> {
        self.caminho.as_deref()
    }

    /// Se este diagnóstico bloqueia a publicação.
    pub fn e_erro(&self) -> bool {
        self.severidade == Severidade::Erro
    }
}

impl fmt::Display for Diagnostico {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.caminho, self.linha, self.coluna) {
            (Some(caminho), _, _) => write!(
                f,
                "{}: {}: {} (no nó {})",
                self.severidade, self.codigo, self.mensagem, caminho
            ),
            (None, Some(linha), Some(coluna)) => write!(
                f,
                "{}: {}: {} (linha {}, coluna {})",
                self.severidade, self.codigo, self.mensagem, linha, coluna
            ),
            _ => write!(f, "{}: {}: {}", self.severidade, self.codigo, self.mensagem),
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn a_lista_fechada_tem_os_41_codigos_do_documento() {
        // Se esta contagem mudar, o §10 do FLUXOS_LINGUAGEM.md mudou junto (ou alguém inventou código).
        assert_eq!(Codigo::TODOS.len(), 41);
        // O compilador emite 40: o FLX65 nasce sobre a IR, na publicação.
        assert_eq!(Codigo::do_compilador().count(), 40);
        assert!(!Codigo::do_compilador().any(|c| c == Codigo::DepartamentoInexistente));
    }

    #[test]
    fn cada_codigo_faz_ida_e_volta_em_texto_e_recusa_inventado() {
        for codigo in Codigo::TODOS {
            let texto = codigo.como_texto();
            assert_eq!(
                Codigo::de_texto(texto),
                Some(*codigo),
                "ida e volta de {texto}"
            );
            assert_eq!(Codigo::from_str(texto), Ok(*codigo));
            assert_eq!(codigo.to_string(), texto);
            let json = serde_json::to_string(codigo).unwrap();
            assert_eq!(json, format!("\"{texto}\""));
        }
        assert_eq!(Codigo::de_texto("FLX99"), None);
        assert!(serde_json::from_str::<Codigo>("\"FLX99\"").is_err());
    }

    #[test]
    fn os_textos_dos_codigos_sao_distintos() {
        let mut vistos: Vec<&str> = Codigo::TODOS.iter().map(|c| c.como_texto()).collect();
        let total = vistos.len();
        vistos.sort_unstable();
        vistos.dedup();
        assert_eq!(vistos.len(), total, "dois códigos com o mesmo texto");
    }

    #[test]
    fn diagnostico_mostra_gravidade_codigo_e_posicao() {
        let d = Diagnostico::erro(
            Codigo::ArgumentoDesconhecido,
            "`obligatorio` não existe",
            3,
            7,
        );
        assert_eq!(
            d.to_string(),
            "erro: FLX12: `obligatorio` não existe (linha 3, coluna 7)"
        );
        assert!(d.e_erro());
        assert!(!Diagnostico::aviso(Codigo::TextoLongo, "longo", 1, 1).e_erro());
    }

    #[test]
    fn diagnostico_da_ir_aponta_o_no_e_nao_aponta_linha() {
        let d = Diagnostico::erro_no(
            Codigo::DestinoInexistente,
            "a ação vai para `depois`, que não existe",
            "etapas[0] (id \"coleta\") > acoes[1] > vai_para",
        );
        assert_eq!(
            d.caminho(),
            Some("etapas[0] (id \"coleta\") > acoes[1] > vai_para")
        );
        assert_eq!((d.linha(), d.coluna()), (None, None));
        assert_eq!(
            d.to_string(),
            "erro: FLX15: a ação vai para `depois`, que não existe (no nó etapas[0] (id \"coleta\") > acoes[1] > vai_para)"
        );
        // No JSON o que não existe não aparece: a API não publica linha 0.
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("linha"), "{json}");
        assert!(json.contains("\"caminho\""), "{json}");
        let voltou: Diagnostico = serde_json::from_str(&json).unwrap();
        assert_eq!(voltou, d);
    }

    #[test]
    fn diagnostico_do_texto_vai_e_volta_em_json_sem_caminho() {
        let d = Diagnostico::aviso(Codigo::CampoNaoUsado, "ninguém usa o campo", 7, 3);
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains("\"linha\":7"), "{json}");
        assert!(!json.contains("caminho"), "{json}");
        assert_eq!(serde_json::from_str::<Diagnostico>(&json).unwrap(), d);
    }
}
