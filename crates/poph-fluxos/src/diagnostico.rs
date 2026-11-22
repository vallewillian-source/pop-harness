//! Diagnósticos de forma do compilador de fluxos (FX-10).
//!
//! A lista é **fechada** e é a de `docs/FLUXOS_LINGUAGEM.md` §10: 26 erros (`FLX01`–`FLX28`) e 14
//! avisos (`FLX51`–`FLX64`). Código novo entra naquele documento antes de entrar aqui. Cada
//! diagnóstico carrega a linha e a coluna do trecho que o provocou (1-based, como o autor conta).
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
}

impl Codigo {
    /// Os 40 códigos, na ordem de `FLUXOS_LINGUAGEM.md` §10.
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
    ];

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

/// Um diagnóstico do compilador, com o ponto exato do script.
///
/// `linha` e `coluna` são 1-based (como o autor conta, e como a API publica). Diagnóstico que não
/// tem trecho (limite de tamanho da fonte) aponta `1:1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostico {
    /// Código da lista fechada.
    pub codigo: Codigo,
    /// Erro ou aviso.
    pub severidade: Severidade,
    /// Mensagem em português, dizendo o que corrigir.
    pub mensagem: String,
    /// Linha do trecho (1-based).
    pub linha: u32,
    /// Coluna do trecho (1-based).
    pub coluna: u32,
}

impl Diagnostico {
    /// Diagnóstico de erro no ponto dado.
    pub fn erro(codigo: Codigo, mensagem: impl Into<String>, linha: u32, coluna: u32) -> Self {
        Self {
            codigo,
            severidade: Severidade::Erro,
            mensagem: mensagem.into(),
            linha,
            coluna,
        }
    }

    /// Diagnóstico de aviso no ponto dado.
    pub fn aviso(codigo: Codigo, mensagem: impl Into<String>, linha: u32, coluna: u32) -> Self {
        Self {
            codigo,
            severidade: Severidade::Aviso,
            mensagem: mensagem.into(),
            linha,
            coluna,
        }
    }

    /// Se este diagnóstico bloqueia a publicação.
    pub fn e_erro(&self) -> bool {
        self.severidade == Severidade::Erro
    }
}

impl fmt::Display for Diagnostico {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {}: {} (linha {}, coluna {})",
            self.severidade, self.codigo, self.mensagem, self.linha, self.coluna
        )
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn a_lista_fechada_tem_os_40_codigos_do_documento() {
        // Se esta contagem mudar, o §10 do FLUXOS_LINGUAGEM.md mudou junto (ou alguém inventou código).
        assert_eq!(Codigo::TODOS.len(), 40);
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
}
