//! A passagem que lê a árvore sintática do script e monta a IR (FX-10).
//!
//! Ler a **árvore** (em vez de executar o script e coletar chamadas) é o que dá linha e coluna em
//! cada diagnóstico e o que permite recusar no lugar certo o que não é declaração — inclusive uma
//! `def` com nome de declaração, que antes estourava em tempo de carga.
//!
//! Nada aqui executa o script. Não há chamada ao interpretador nem alocação guiada por dado do
//! autor: `"x" * 1000000000` no topo do arquivo é erro de forma em microssegundos, não um pico de
//! 2 GB. A única avaliação de Starlark do compilador é a carga do módulo
//! ([`crate::linguagem::carregar_modulo`]), que roda **depois** de a forma estar válida e com os
//! limites ligados.
//!
//! Duas passagens, de propósito:
//!
//! 1. **Coleta:** cada declaração vira uma estrutura bruta, com a posição de cada pedaço. É onde a
//!    forma local é conferida (argumento desconhecido, tipo errado, id inválido, aridade do
//!    combinador).
//! 2. **Fechamento:** com todas as declarações conhecidas, resolve destino (`inicio`, `vai_para`,
//!    `entao`, `senao`), confere regra contra etapa/campo, detecta ciclo entre decisões e emite os
//!    avisos de alcance e de uso. Uma decisão pode citar etapa declarada depois dela, e uma `def`
//!    usada antes de ser definida: só nesta passagem isso é sabido.

use std::collections::{BTreeMap, BTreeSet};

use poph_types::{ActionActor, FieldKind};
use starlark::codemap::Span;
use starlark::syntax::AstModule;
use starlark::syntax::ast::{
    Argument, AstAssignTarget, AstExpr, AstIdent, AstLiteral, AstNoPayload, AstStmt, CallArgsP,
    Clause, DefP, Expr, Parameter, Stmt,
};

use crate::diagnostico::{Codigo, Diagnostico};
use crate::ir::{
    Acao, Campo, Decisao, Def, Destino, Efeito, Etapa, Fim, Fluxo, Gatilho, Ir, Regra,
    SCHEMA_VERSAO,
};
use crate::linguagem::nome_permitido;

/// Argumentos de uma chamada do script.
type ArgsAst<'a> = &'a CallArgsP<AstNoPayload>;

/// Teto de nós (etapas + decisões + fins), `FLUXOS_LINGUAGEM.md` §3.4.
pub const MAX_NOS: usize = 60;
/// Teto recomendado de `quando`/`nao_quando`.
const MAX_TEXTO_GATILHO: usize = 400;
/// Teto recomendado de `instrucao`.
const MAX_TEXTO_INSTRUCAO: usize = 1000;
/// Menor id aceito.
const ID_MIN: usize = 2;
/// Maior id aceito.
const ID_MAX: usize = 64;

/// Argumentos de `fluxo(...)`.
const ARGS_FLUXO: &[&str] = &["id", "nome", "descricao", "inicio", "gatilho"];
/// Argumentos de `gatilho(...)`.
const ARGS_GATILHO: &[&str] = &["quando", "nao_quando", "exemplos"];
/// Argumentos de `etapa(...)`.
const ARGS_ETAPA: &[&str] = &[
    "id",
    "nome",
    "instrucao",
    "atores",
    "campos",
    "acoes",
    "ao_entrar",
];
/// Argumentos nomeados de `campo(...)`, além dos dois posicionais.
const ARGS_CAMPO: &[&str] = &["id", "rotulo", "tipo", "obrigatorio", "opcoes", "ajuda"];
/// Argumentos nomeados de `acao(...)`, além dos dois posicionais.
const ARGS_ACAO: &[&str] = &["id", "rotulo", "vai_para", "exige", "ator", "efeitos"];
/// Argumentos nomeados de `fim(...)`, além do id posicional.
const ARGS_FIM: &[&str] = &["id", "resultado", "encerra_atendimento"];
/// Argumentos de `decisao(...)`.
const ARGS_DECISAO: &[&str] = &["id", "se", "entao", "senao"];
/// Argumentos de `notificar_whatsapp(...)`.
const ARGS_NOTIFICAR: &[&str] = &["para", "mensagem"];

/// Ponto do script (1-based, como o autor conta).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pos {
    linha: u32,
    coluna: u32,
}

/// Um item de uma chamada: nomeado ou posicional.
struct Item<'a> {
    nome: Option<String>,
    expr: &'a AstExpr,
}

/// Destino antes de saber o que existe no fluxo.
#[derive(Clone)]
enum DestinoBruto {
    /// Id de etapa ou decisão (ainda não resolvido).
    Alvo { id: String, pos: Pos },
    /// `fim(...)` já lido.
    Fim { fim: Fim, pos: Pos },
    /// Não foi declarado ou foi recusado: já diagnosticado.
    Invalido,
}

/// Regra antes de saber o que existe no fluxo.
#[derive(Clone)]
enum RegraBruta {
    /// `igual`, `diferente`, `maior`, `menor`, `contem`, `vazio`.
    Folha { regra: Regra, pos: Pos },
    /// `funcao("nome")`.
    Funcao { nome: String, pos: Pos },
    /// `e`, `ou`, `nao`.
    Grupo {
        regra: Regra,
        filhos: Vec<RegraBruta>,
        pos: Pos,
    },
    /// Recusada: já diagnosticada.
    Invalida,
}

/// Ação com o destino ainda bruto.
#[derive(Clone)]
struct AcaoBruta {
    acao: Acao,
    destino: DestinoBruto,
}

/// Etapa coletada, com as posições que os avisos precisam.
#[derive(Clone)]
struct EtapaColetada {
    etapa: Etapa,
    acoes: Vec<AcaoBruta>,
    pos: Pos,
    pos_campos: BTreeMap<String, Pos>,
}

/// Decisão coletada.
#[derive(Clone)]
struct DecisaoColetada {
    id: String,
    regra: RegraBruta,
    entao: DestinoBruto,
    senao: DestinoBruto,
    pos: Pos,
}

/// Efeito com a posição do texto e de qual etapa veio.
struct EfeitoColetado {
    efeito: Efeito,
    pos_texto: Pos,
    etapa: String,
    ao_entrar: bool,
}

/// O que as ações e as regras usam de cada etapa.
#[derive(Default, Clone)]
struct UsoCampos {
    exigidos: BTreeSet<String>,
    lidos: BTreeSet<String>,
}

/// Estado da compilação de forma.
struct Forma<'a> {
    fonte: &'a str,
    modulo: &'a AstModule,
    diags: Vec<Diagnostico>,
    fluxo: Option<Fluxo>,
    pos_fluxo: Pos,
    gatilho: Option<Gatilho>,
    inicio: Option<(String, Pos)>,
    etapas: Vec<EtapaColetada>,
    idx_etapas: BTreeMap<String, usize>,
    decisoes: Vec<DecisaoColetada>,
    idx_decisoes: BTreeMap<String, usize>,
    fins: Vec<Fim>,
    pos_fins: BTreeMap<String, Pos>,
    defs: Vec<Def>,
    pos_defs: BTreeMap<String, Pos>,
    efeitos: Vec<EfeitoColetado>,
    usos: BTreeMap<String, UsoCampos>,
}

/// Compila a forma: monta a IR e devolve os diagnósticos.
///
/// `modulo` é o resultado do parse (o chamador já tratou tamanho e erro de sintaxe). A IR só sai
/// quando **não há erro**; com aviso ela sai normal.
pub(crate) fn compila(fonte: &str, modulo: &AstModule) -> (Option<Ir>, Vec<Diagnostico>) {
    let um = Pos {
        linha: 1,
        coluna: 1,
    };
    let mut forma = Forma {
        fonte,
        modulo,
        diags: Vec::new(),
        fluxo: None,
        pos_fluxo: um,
        gatilho: None,
        inicio: None,
        etapas: Vec::new(),
        idx_etapas: BTreeMap::new(),
        decisoes: Vec::new(),
        idx_decisoes: BTreeMap::new(),
        fins: Vec::new(),
        pos_fins: BTreeMap::new(),
        defs: Vec::new(),
        pos_defs: BTreeMap::new(),
        efeitos: Vec::new(),
        usos: BTreeMap::new(),
    };
    forma.coleta_topo();
    let ir = forma.fecha();
    (ir, forma.diags)
}

impl<'a> Forma<'a> {
    // ---------------------------------------------------------------- utilidades

    fn pos(&self, span: Span) -> Pos {
        let resolvido = self.modulo.file_span(span).resolve_span();
        Pos {
            linha: resolvido.begin.line as u32 + 1,
            coluna: resolvido.begin.column as u32 + 1,
        }
    }

    fn erro(&mut self, codigo: Codigo, mensagem: impl Into<String>, pos: Pos) {
        self.diags
            .push(Diagnostico::erro(codigo, mensagem, pos.linha, pos.coluna));
    }

    fn aviso(&mut self, codigo: Codigo, mensagem: impl Into<String>, pos: Pos) {
        self.diags
            .push(Diagnostico::aviso(codigo, mensagem, pos.linha, pos.coluna));
    }

    fn tem_erro(&self) -> bool {
        self.diags.iter().any(Diagnostico::e_erro)
    }

    // ---------------------------------------------------------------- passagem 1: topo

    fn coleta_topo(&mut self) {
        self.coleta_lista(self.modulo.statement());
    }

    fn coleta_lista(&mut self, stmt: &'a AstStmt) {
        let itens: Vec<&'a AstStmt> = match &stmt.node {
            Stmt::Statements(itens) => itens.iter().collect(),
            _ => vec![stmt],
        };
        for item in itens {
            self.coleta_item(item);
        }
    }

    fn coleta_item(&mut self, item: &'a AstStmt) {
        let pos = self.pos(item.span);
        match &item.node {
            Stmt::Def(def) => self.coleta_def(def, item, pos),
            Stmt::Expression(expr) => match &expr.node {
                Expr::Call(alvo, args) => {
                    let nome = match &alvo.node {
                        Expr::Identifier(ident) => ident.node.ident.clone(),
                        _ => {
                            self.erro(
                                Codigo::TopoInvalido,
                                "No topo do arquivo só pode haver `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`; isto é uma chamada de atributo.",
                                pos,
                            );
                            return;
                        }
                    };
                    match nome.as_str() {
                        "fluxo" => self.coleta_fluxo(args, pos),
                        "etapa" => self.coleta_etapa(args, pos),
                        "decisao" => self.coleta_decisao(args, pos),
                        outro => self.erro(
                            Codigo::TopoInvalido,
                            format!(
                                "`{outro}(...)` não é declaração de fluxo: no topo do arquivo só pode haver `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`."
                            ),
                            pos,
                        ),
                    }
                }
                _ => self.erro(
                    Codigo::TopoInvalido,
                    "No topo do arquivo só pode haver `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`. Tire esta expressão solta.",
                    pos,
                ),
            },
            Stmt::Assign(_) => self.erro(
                Codigo::TopoInvalido,
                "No topo do arquivo não existe atribuição (`x = ...`): só `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`.",
                pos,
            ),
            Stmt::AssignModify(..) => self.erro(
                Codigo::TopoInvalido,
                "No topo do arquivo não existe atribuição (`x += ...`): só `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`.",
                pos,
            ),
            Stmt::For(_) => self.erro(
                Codigo::Dialeto,
                "Laço (`for`) não existe no topo de um fluxo: só `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`.",
                pos,
            ),
            Stmt::If(..) | Stmt::IfElse(..) => self.erro(
                Codigo::Dialeto,
                "Condição (`if`) não existe no topo de um fluxo: a bifurcação do atendimento é `decisao(...)`.",
                pos,
            ),
            Stmt::Load(_) => self.erro(
                Codigo::Dialeto,
                "`load` não faz parte da linguagem de fluxos: um fluxo não importa outro arquivo.",
                pos,
            ),
            Stmt::Return(_) | Stmt::Break | Stmt::Continue | Stmt::Pass | Stmt::Statements(_) => self.erro(
                Codigo::TopoInvalido,
                "No topo do arquivo só pode haver `fluxo(...)`, `etapa(...)`, `decisao(...)` e `def`.",
                pos,
            ),
        }
    }

    /// Lê os argumentos de uma chamada, recusando `*`/`**`.
    fn itens_de(&mut self, args: ArgsAst<'a>, onde: &str, pos: Pos) -> Vec<Item<'a>> {
        let mut itens = Vec::new();
        for arg in &args.args {
            match &arg.node {
                Argument::Positional(expr) => itens.push(Item { nome: None, expr }),
                Argument::Named(nome, expr) => itens.push(Item {
                    nome: Some(nome.node.clone()),
                    expr,
                }),
                Argument::Args(_) | Argument::KwArgs(_) => self.erro(
                    Codigo::ArgumentoDesconhecido,
                    format!(
                        "{onde} não aceita `*`/`**` na chamada: escreva os argumentos um a um."
                    ),
                    pos,
                ),
            }
        }
        itens
    }

    // ---------------------------------------------------------------- fluxo

    fn coleta_fluxo(&mut self, args: ArgsAst<'a>, pos: Pos) {
        if self.fluxo.is_some() {
            self.erro(
                Codigo::FluxoAusente,
                "O arquivo declara `fluxo(...)` mais de uma vez. Um arquivo é um fluxo: deixe só um.",
                pos,
            );
            return;
        }
        self.pos_fluxo = pos;
        let itens = self.itens_de(args, "`fluxo`", pos);
        let mut leitor = Leitor::novo("`fluxo`", pos, ARGS_FLUXO, itens);
        let id = leitor.texto(self, "id");
        let nome = leitor.texto(self, "nome");
        let descricao = leitor.texto(self, "descricao");
        let inicio = leitor.texto(self, "inicio");
        let gatilho = leitor.item(self, "gatilho");
        let pos_id = leitor.pos_de("id").unwrap_or(pos);
        let pos_inicio = leitor.pos_de("inicio");
        leitor.fecha(self);

        if let Some(id) = &id
            && !id_valido(id)
        {
            self.erro(
                Codigo::IdInvalido,
                format!(
                    "O id `{id}` não é válido: use minúsculas, números e `_`, de {ID_MIN} a {ID_MAX} caracteres, por exemplo `agendar_visita`."
                ),
                pos_id,
            );
        }
        if let Some(inicio) = &inicio {
            self.inicio = Some((inicio.clone(), pos_inicio.unwrap_or(pos)));
        }
        if let Some(expr) = gatilho {
            let pos_gatilho = self.pos(expr.span);
            self.coleta_gatilho(expr, pos_gatilho);
        }
        self.fluxo = Some(Fluxo {
            id: id.unwrap_or_default(),
            nome: nome.unwrap_or_default(),
            descricao,
        });
    }

    fn coleta_gatilho(&mut self, expr: &'a AstExpr, pos: Pos) {
        let Some((nome, itens)) = self.como_chamada(expr, "o `gatilho` do fluxo") else {
            return;
        };
        if nome != "gatilho" {
            self.erro(
                Codigo::TipoErrado,
                format!("O `gatilho` do fluxo precisa ser `gatilho(quando = ..., nao_quando = ..., exemplos = [...])`, não `{nome}(...)`."),
                pos,
            );
            return;
        }
        let mut leitor = Leitor::novo("`gatilho`", pos, ARGS_GATILHO, itens);
        let quando = leitor.texto(self, "quando");
        let nao_quando = leitor.texto(self, "nao_quando");
        let exemplos_expr = leitor.item(self, "exemplos");
        let pos_quando = leitor.pos_de("quando").unwrap_or(pos);
        let pos_nao_quando = leitor.pos_de("nao_quando").unwrap_or(pos);
        let exemplos = match exemplos_expr {
            Some(expr) => self.lista_texto(expr, "`exemplos` do gatilho"),
            None => Vec::new(),
        };
        leitor.fecha(self);

        for (chave, texto, pos_texto) in [
            ("quando", quando.as_deref(), pos_quando),
            ("nao_quando", nao_quando.as_deref(), pos_nao_quando),
        ] {
            let Some(texto) = texto else { continue };
            if !variaveis(texto).is_empty() {
                self.erro(
                    Codigo::InterpolacaoProibida,
                    format!("`gatilho.{chave}` tem `{{...}}`, mas esse texto não interpola: o Jev lê o texto como está."),
                    pos_texto,
                );
            }
            if texto.chars().count() > MAX_TEXTO_GATILHO {
                self.aviso(
                    Codigo::TextoLongo,
                    format!(
                        "`gatilho.{chave}` tem {} caracteres; o recomendado é até {MAX_TEXTO_GATILHO}.",
                        texto.chars().count()
                    ),
                    pos_texto,
                );
            }
        }
        if exemplos.is_empty() {
            self.aviso(
                Codigo::GatilhoSemExemplos,
                "O gatilho não tem `exemplos`: com 2 ou 3 falas típicas do cliente o Jev acerta mais.",
                pos,
            );
        }
        if nao_quando.is_none() {
            self.aviso(
                Codigo::GatilhoSemNegativo,
                "O gatilho não tem `nao_quando`: diga em que situação o fluxo NÃO deve começar.",
                pos,
            );
        }
        self.gatilho = Some(Gatilho {
            quando: quando.unwrap_or_default(),
            nao_quando,
            exemplos,
        });
    }

    // ---------------------------------------------------------------- etapa

    fn coleta_etapa(&mut self, args: ArgsAst<'a>, pos: Pos) {
        let itens = self.itens_de(args, "`etapa`", pos);
        let mut leitor = Leitor::novo("`etapa`", pos, ARGS_ETAPA, itens);
        let id = leitor.texto(self, "id");
        let nome = leitor.texto(self, "nome");
        let instrucao = leitor.texto(self, "instrucao");
        let atores_expr = leitor.item(self, "atores");
        let campos_expr = leitor.item(self, "campos");
        let acoes_expr = leitor.item(self, "acoes");
        let ao_entrar_expr = leitor.item(self, "ao_entrar");
        let pos_id = leitor.pos_de("id").unwrap_or(pos);
        let pos_instrucao = leitor.pos_de("instrucao").unwrap_or(pos);
        leitor.fecha(self);

        let id = id.unwrap_or_default();
        if !id.is_empty() {
            if !id_valido(&id) {
                self.erro(
                    Codigo::IdInvalido,
                    format!("O id de etapa `{id}` não é válido: use minúsculas, números e `_`, de {ID_MIN} a {ID_MAX} caracteres."),
                    pos_id,
                );
            }
            if self.id_ocupado(&id) {
                self.erro(
                    Codigo::IdRepetido,
                    format!("O id `{id}` está declarado mais de uma vez. Cada etapa e cada decisão tem id único no fluxo."),
                    pos_id,
                );
            }
        }
        if let Some(texto) = &instrucao {
            if !variaveis(texto).is_empty() {
                self.erro(
                    Codigo::InterpolacaoProibida,
                    format!(
                        "A `instrucao` da etapa `{id}` tem `{{...}}`, mas instrução não interpola: o agente lê o texto como está. Use variável só em `mensagem_ao_cliente` e `notificar_whatsapp`."
                    ),
                    pos_instrucao,
                );
            }
            if texto.chars().count() > MAX_TEXTO_INSTRUCAO {
                self.aviso(
                    Codigo::TextoLongo,
                    format!(
                        "A `instrucao` da etapa `{id}` tem {} caracteres; o recomendado é até {MAX_TEXTO_INSTRUCAO}, porque entra no prompt a cada turno.",
                        texto.chars().count()
                    ),
                    pos_instrucao,
                );
            }
        }

        let mut atores_acao: Vec<ActionActor> = Vec::new();
        if let Some(expr) = atores_expr {
            let pos_atores = self.pos(expr.span);
            for ator in self.lista_texto(expr, &format!("`atores` da etapa `{id}`")) {
                match ActionActor::from_text(&ator) {
                    Ok(ator) => atores_acao.push(ator),
                    Err(_) => self.erro(
                        Codigo::RegraOuAtorInvalido,
                        format!("A etapa `{id}` tem o ator `{ator}`; use `agente` ou `atendente` (o cliente não é ator)."),
                        pos_atores,
                    ),
                }
            }
        }
        if atores_acao.is_empty() {
            atores_acao.push(ActionActor::Agent);
        }
        let mut unicos: Vec<ActionActor> = Vec::new();
        for ator in atores_acao {
            if !unicos.contains(&ator) {
                unicos.push(ator);
            }
        }
        let atores_acao = unicos;

        self.usos.insert(id.clone(), UsoCampos::default());

        let (campos, pos_campos) = match campos_expr {
            Some(expr) => self.coleta_campos(expr, &id),
            None => (Vec::new(), BTreeMap::new()),
        };
        let acoes = match acoes_expr {
            Some(expr) => self.coleta_acoes(expr, &id, &campos, &atores_acao),
            None => Vec::new(),
        };
        let mut ao_entrar = Vec::new();
        if let Some(expr) = ao_entrar_expr {
            let onde = format!("`ao_entrar` da etapa `{id}`");
            for efeito in self.coleta_efeitos(expr, &onde) {
                ao_entrar.push(efeito.efeito.clone());
                self.efeitos.push(EfeitoColetado {
                    efeito: efeito.efeito,
                    pos_texto: efeito.pos_texto,
                    etapa: id.clone(),
                    ao_entrar: true,
                });
            }
        }

        // Ator listado na etapa sem nenhuma ação dele: na prática ele não tem o que fazer aqui.
        // Só vale quando toda ação é restrita (ação sem `ator` qualquer um dos atores executa).
        if !acoes.is_empty() && acoes.iter().all(|a| a.acao.ator.is_some()) {
            for ator in &atores_acao {
                if acoes.iter().any(|a| a.acao.ator == Some(*ator)) {
                    continue;
                }
                let pos_atores = atores_expr.map(|expr| self.pos(expr.span)).unwrap_or(pos);
                self.aviso(
                    Codigo::AtorSemAcao,
                    format!(
                        "A etapa `{id}` permite o `{}`, mas nenhuma ação é dele: na prática ele não tem o que fazer aqui.",
                        ator.as_text()
                    ),
                    pos_atores,
                );
            }
        }

        let indice = self.etapas.len();
        self.idx_etapas.insert(id.clone(), indice);
        self.etapas.push(EtapaColetada {
            etapa: Etapa {
                id,
                nome: nome.unwrap_or_default(),
                instrucao,
                atores_acao,
                campos,
                acoes: Vec::new(),
                ao_entrar,
            },
            acoes,
            pos,
            pos_campos,
        });
    }

    fn id_ocupado(&self, id: &str) -> bool {
        self.idx_etapas.contains_key(id) || self.idx_decisoes.contains_key(id)
    }

    fn coleta_campos(
        &mut self,
        expr: &'a AstExpr,
        etapa: &str,
    ) -> (Vec<Campo>, BTreeMap<String, Pos>) {
        let mut campos = Vec::new();
        let mut posicoes = BTreeMap::new();
        let mut vistos: BTreeSet<String> = BTreeSet::new();
        let onde_lista = format!("`campos` da etapa `{etapa}`");
        for item in self.lista_itens(expr, &onde_lista) {
            let pos = self.pos(item.span);
            let Some((nome, itens)) =
                self.como_chamada(item, &format!("um item de `campos` da etapa `{etapa}`"))
            else {
                continue;
            };
            if nome != "campo" {
                self.erro(
                    Codigo::TipoErrado,
                    format!(
                        "`campos` da etapa `{etapa}` aceita só `campo(...)`, não `{nome}(...)`."
                    ),
                    pos,
                );
                continue;
            }
            let mut leitor = Leitor::novo("`campo`", pos, ARGS_CAMPO, itens);
            let id = leitor.posicional_texto(self, 0);
            let rotulo = leitor.posicional_texto(self, 1);
            let tipo_expr = leitor.item(self, "tipo");
            let obrigatorio_expr = leitor.item(self, "obrigatorio");
            let opcoes_expr = leitor.item(self, "opcoes");
            let ajuda = leitor.texto(self, "ajuda");
            let pos_id = leitor.pos_de("__pos0").unwrap_or(pos);
            let pos_rotulo = leitor.pos_de("__pos1").unwrap_or(pos);
            let pos_tipo = leitor.pos_de("tipo").unwrap_or(pos);
            let _pos_obrigatorio = leitor.pos_de("obrigatorio").unwrap_or(pos);
            let pos_opcoes = leitor.pos_de("opcoes").unwrap_or(pos);
            leitor.fecha(self);

            let id = id.unwrap_or_default();
            if !id.is_empty() && !id_valido(&id) {
                self.erro(
                    Codigo::IdInvalido,
                    format!("O id do campo `{id}` na etapa `{etapa}` não é válido: use minúsculas, números e `_`, de {ID_MIN} a {ID_MAX} caracteres."),
                    pos_id,
                );
            }
            let mut repetido = false;
            if !id.is_empty() && !vistos.insert(id.clone()) {
                self.erro(
                    Codigo::IdRepetido,
                    format!("O campo `{id}` está declarado duas vezes na etapa `{etapa}`."),
                    pos_id,
                );
                repetido = true;
            }
            if rotulo.is_none() {
                self.erro(
                    Codigo::ArgumentoAusente,
                    format!(
                        "O campo `{etapa}.{id}` não recebeu o rótulo (segundo argumento): `campo(\"{id}\", \"Rótulo\")`."
                    ),
                    pos_rotulo,
                );
            }
            let tipo = match tipo_expr {
                Some(expr) => {
                    let bruto = self
                        .texto(expr, &format!("`tipo` do campo `{etapa}.{id}`"))
                        .unwrap_or_default();
                    match FieldKind::from_text(&bruto) {
                        Ok(tipo) => tipo,
                        Err(_) => {
                            self.erro(
                                Codigo::TipoDesconhecido,
                                format!(
                                    "O campo `{etapa}.{id}` tem tipo `{bruto}`; use texto, numero, data, booleano, opcao ou anexo."
                                ),
                                pos_tipo,
                            );
                            FieldKind::Text
                        }
                    }
                }
                None => FieldKind::Text,
            };
            let obrigatorio = match obrigatorio_expr {
                Some(expr) => self
                    .booleano(expr, &format!("`obrigatorio` do campo `{etapa}.{id}`"))
                    .unwrap_or(false),
                None => false,
            };
            let mut opcoes = Vec::new();
            if let Some(expr) = opcoes_expr {
                opcoes = self.lista_texto(expr, &format!("`opcoes` do campo `{etapa}.{id}`"));
                let mut vistas = BTreeSet::new();
                for opcao in &opcoes {
                    if !vistas.insert(opcao.clone()) {
                        self.erro(
                            Codigo::OpcoesInvalidas,
                            format!("O campo `{etapa}.{id}` repete a opção `{opcao}`."),
                            pos_opcoes,
                        );
                    }
                }
            }
            if tipo == FieldKind::Option && opcoes.len() < 2 {
                self.erro(
                    Codigo::OpcoesInvalidas,
                    format!(
                        "O campo `{etapa}.{id}` é `opcao` e precisa de pelo menos duas `opcoes`."
                    ),
                    if opcoes_expr.is_some() {
                        pos_opcoes
                    } else {
                        pos_id
                    },
                );
            }
            if tipo != FieldKind::Option && !opcoes.is_empty() {
                self.erro(
                    Codigo::OpcoesInvalidas,
                    format!(
                        "O campo `{etapa}.{id}` tem `opcoes`, mas o tipo é `{}`: `opcoes` só vale com `tipo = \"opcao\"`.",
                        tipo.as_text()
                    ),
                    pos_opcoes,
                );
            }
            if repetido {
                continue;
            }
            posicoes.insert(id.clone(), pos_id);
            campos.push(Campo {
                id,
                rotulo: rotulo.unwrap_or_default(),
                tipo,
                obrigatorio,
                opcoes,
                ajuda,
            });
        }
        (campos, posicoes)
    }

    fn coleta_acoes(
        &mut self,
        expr: &'a AstExpr,
        etapa: &str,
        campos: &[Campo],
        atores: &[ActionActor],
    ) -> Vec<AcaoBruta> {
        let mut acoes = Vec::new();
        let mut vistos: BTreeSet<String> = BTreeSet::new();
        let ids_campos: BTreeSet<&str> = campos.iter().map(|c| c.id.as_str()).collect();
        let onde_lista = format!("`acoes` da etapa `{etapa}`");
        for item in self.lista_itens(expr, &onde_lista) {
            let pos = self.pos(item.span);
            let Some((nome, itens)) =
                self.como_chamada(item, &format!("um item de `acoes` da etapa `{etapa}`"))
            else {
                continue;
            };
            if nome != "acao" {
                self.erro(
                    Codigo::TipoErrado,
                    format!("`acoes` da etapa `{etapa}` aceita só `acao(...)`, não `{nome}(...)`."),
                    pos,
                );
                continue;
            }
            let mut leitor = Leitor::novo("`acao`", pos, ARGS_ACAO, itens);
            let id = leitor.posicional_texto(self, 0);
            let rotulo = leitor.posicional_texto(self, 1);
            let vai_para = leitor.item(self, "vai_para");
            let exige_expr = leitor.item(self, "exige");
            let ator_expr = leitor.item(self, "ator");
            let efeitos_expr = leitor.item(self, "efeitos");
            let pos_vai_para = leitor.pos_de("vai_para").unwrap_or(pos);
            let pos_exige = leitor.pos_de("exige").unwrap_or(pos);
            let pos_ator = leitor.pos_de("ator").unwrap_or(pos);
            leitor.fecha(self);

            let id = id.unwrap_or_default();
            if !id.is_empty() && !id_valido(&id) {
                self.erro(
                    Codigo::IdInvalido,
                    format!("O id da ação `{id}` na etapa `{etapa}` não é válido: use minúsculas, números e `_`, de {ID_MIN} a {ID_MAX} caracteres."),
                    pos,
                );
            }
            if rotulo.is_none() {
                self.erro(
                    Codigo::ArgumentoAusente,
                    format!(
                        "A ação `{etapa}.{id}` não recebeu o rótulo (segundo argumento): `acao(\"{id}\", \"Rótulo\", vai_para = ...)`."
                    ),
                    pos,
                );
            }
            let mut exige = Vec::new();
            if let Some(expr) = exige_expr {
                let onde_exige = format!("`exige` da ação `{etapa}.{id}`");
                for campo in self.lista_texto(expr, &onde_exige) {
                    if !ids_campos.contains(campo.as_str()) {
                        self.erro(
                            Codigo::ExigeInexistente,
                            format!("A ação `{etapa}.{id}` exige o campo `{campo}`, que não existe na etapa `{etapa}`."),
                            pos_exige,
                        );
                        continue;
                    }
                    if let Some(uso) = self.usos.get_mut(etapa) {
                        uso.exigidos.insert(campo.clone());
                    }
                    exige.push(campo);
                }
            }
            let ator = match ator_expr {
                Some(expr) => {
                    let bruto = self
                        .texto(expr, &format!("`ator` da ação `{etapa}.{id}`"))
                        .unwrap_or_default();
                    match ActionActor::from_text(&bruto) {
                        Ok(ator) => {
                            if !atores.contains(&ator) {
                                self.erro(
                                    Codigo::RegraOuAtorInvalido,
                                    format!(
                                        "A ação `{etapa}.{id}` é do ator `{bruto}`, que não está em `atores` da etapa `{etapa}`. Inclua `{bruto}` em `atores` ou tire o `ator` da ação."
                                    ),
                                    pos_ator,
                                );
                            }
                            Some(ator)
                        }
                        Err(_) => {
                            self.erro(
                                Codigo::RegraOuAtorInvalido,
                                format!("A ação `{etapa}.{id}` tem `ator = \"{bruto}\"`; use `agente` ou `atendente` (ou deixe vazio)."),
                                pos_ator,
                            );
                            None
                        }
                    }
                }
                None => None,
            };
            let destino = match vai_para {
                Some(expr) => self.destino(expr, &format!("a ação `{etapa}.{id}`")),
                None => {
                    self.erro(
                        Codigo::ArgumentoAusente,
                        format!("A ação `{etapa}.{id}` não recebeu `vai_para`: diga para qual etapa, decisão ou `fim(...)` ela leva."),
                        pos_vai_para,
                    );
                    DestinoBruto::Invalido
                }
            };
            let mut efeitos = Vec::new();
            if let Some(expr) = efeitos_expr {
                let onde_efeitos = format!("`efeitos` da ação `{etapa}.{id}`");
                for efeito in self.coleta_efeitos(expr, &onde_efeitos) {
                    efeitos.push(efeito.efeito.clone());
                    self.efeitos.push(EfeitoColetado {
                        efeito: efeito.efeito,
                        pos_texto: efeito.pos_texto,
                        etapa: etapa.to_owned(),
                        ao_entrar: false,
                    });
                }
            }
            let mut segue = true;
            if !id.is_empty() && !vistos.insert(id.clone()) {
                self.erro(
                    Codigo::IdRepetido,
                    format!("A ação `{id}` está declarada duas vezes na etapa `{etapa}`."),
                    pos,
                );
                segue = false;
            }
            if !segue {
                continue;
            }
            acoes.push(AcaoBruta {
                acao: Acao {
                    id,
                    rotulo: rotulo.unwrap_or_default(),
                    exige,
                    ator,
                    vai_para: Destino::Etapa { id: String::new() },
                    efeitos,
                },
                destino,
            });
        }
        acoes
    }

    fn coleta_efeitos(&mut self, expr: &'a AstExpr, onde: &str) -> Vec<EfeitoLido> {
        let mut saida = Vec::new();
        for item in self.lista_itens(expr, onde) {
            let pos = self.pos(item.span);
            let Some((nome, itens)) = self.como_chamada(item, onde) else {
                continue;
            };
            let mut pos_texto_final = pos;
            let efeito = match nome.as_str() {
                "mensagem_ao_cliente" => {
                    let mut leitor = Leitor::novo("`mensagem_ao_cliente`", pos, &[], itens);
                    let texto = leitor.posicional_texto(self, 0);
                    let pos_texto = leitor.pos_de("__pos0").unwrap_or(pos);
                    leitor.fecha(self);
                    let Some(texto) = texto else {
                        self.erro(
                            Codigo::ArgumentoAusente,
                            format!("{onde} tem `mensagem_ao_cliente()` sem o texto: `mensagem_ao_cliente(\"Texto enviado ao cliente\")`."),
                            pos,
                        );
                        continue;
                    };
                    self.confere_formato_das_variaveis(&texto, onde, pos_texto);
                    pos_texto_final = pos_texto;
                    Efeito::MensagemAoCliente { texto }
                }
                "notificar_whatsapp" => {
                    let mut leitor =
                        Leitor::novo("`notificar_whatsapp`", pos, ARGS_NOTIFICAR, itens);
                    let para = leitor.texto(self, "para");
                    let mensagem = leitor.texto(self, "mensagem");
                    let pos_para = leitor.pos_de("para").unwrap_or(pos);
                    let pos_mensagem = leitor.pos_de("mensagem").unwrap_or(pos);
                    leitor.fecha(self);
                    let para = match para {
                        Some(para) => {
                            if para.len() < 10 || !para.chars().all(|c| c.is_ascii_digit()) {
                                self.erro(
                                    Codigo::NumeroInvalido,
                                    format!("O número `{para}` não serve: use dígitos com DDI, por exemplo `5521999990000`."),
                                    pos_para,
                                );
                            }
                            para
                        }
                        None => {
                            self.erro(
                                Codigo::ArgumentoAusente,
                                format!("{onde} tem `notificar_whatsapp` sem `para`: indique o número que recebe a notificação."),
                                pos,
                            );
                            String::new()
                        }
                    };
                    let mensagem = match mensagem {
                        Some(mensagem) => {
                            pos_texto_final = pos_mensagem;
                            self.confere_formato_das_variaveis(&mensagem, onde, pos_mensagem);
                            if variaveis(&mensagem).is_empty() {
                                self.aviso(
                                    Codigo::NotificacaoSemVariavel,
                                    format!("{onde} notifica a equipe sem nenhuma variável: ela recebe sempre o mesmo texto, sem saber de qual atendimento se trata."),
                                    pos_mensagem,
                                );
                            }
                            mensagem
                        }
                        None => {
                            self.erro(
                                Codigo::ArgumentoAusente,
                                format!("{onde} tem `notificar_whatsapp` sem `mensagem`: escreva o texto da notificação."),
                                pos,
                            );
                            String::new()
                        }
                    };
                    Efeito::NotificarWhatsapp { para, mensagem }
                }
                "encaminhar" => {
                    let mut leitor = Leitor::novo("`encaminhar`", pos, &[], itens);
                    let departamento = leitor.posicional_texto(self, 0);
                    leitor.fecha(self);
                    match departamento {
                        Some(departamento) => Efeito::Encaminhar { departamento },
                        None => {
                            self.erro(
                                Codigo::ArgumentoAusente,
                                format!("{onde} tem `encaminhar()` sem o departamento: `encaminhar(\"Logística\")`."),
                                pos,
                            );
                            continue;
                        }
                    }
                }
                outro => {
                    self.erro(
                        Codigo::TopoInvalido,
                        format!("{onde} usa o efeito `{outro}(...)`; use `mensagem_ao_cliente`, `notificar_whatsapp` ou `encaminhar`."),
                        pos,
                    );
                    continue;
                }
            };
            saida.push(EfeitoLido {
                efeito,
                pos_texto: pos_texto_final,
            });
        }
        saida
    }

    // ---------------------------------------------------------------- decisão

    fn coleta_decisao(&mut self, args: ArgsAst<'a>, pos: Pos) {
        let itens = self.itens_de(args, "`decisao`", pos);
        let mut leitor = Leitor::novo("`decisao`", pos, ARGS_DECISAO, itens);
        let id = leitor.texto(self, "id");
        let se_expr = leitor.item(self, "se");
        let entao_expr = leitor.item(self, "entao");
        let senao_expr = leitor.item(self, "senao");
        let pos_id = leitor.pos_de("id").unwrap_or(pos);
        let pos_se = leitor.pos_de("se").unwrap_or(pos);
        let pos_entao = leitor.pos_de("entao").unwrap_or(pos);
        let pos_senao = leitor.pos_de("senao").unwrap_or(pos);
        leitor.fecha(self);

        let id = id.unwrap_or_default();
        if !id.is_empty() {
            if !id_valido(&id) {
                self.erro(
                    Codigo::IdInvalido,
                    format!("O id de decisão `{id}` não é válido: use minúsculas, números e `_`, de {ID_MIN} a {ID_MAX} caracteres."),
                    pos_id,
                );
            }
            if self.id_ocupado(&id) {
                self.erro(
                    Codigo::IdRepetido,
                    format!("O id `{id}` está declarado mais de uma vez. Cada etapa e cada decisão tem id único no fluxo."),
                    pos_id,
                );
            }
        }
        let regra = match se_expr {
            Some(expr) => self.regra(expr, &format!("a decisão `{id}`")),
            None => {
                self.erro(
                    Codigo::ArgumentoAusente,
                    format!("A decisão `{id}` não recebeu `se`: escreva a regra, por exemplo `se = igual(\"coleta.periodo\", \"manhã\")`."),
                    pos_se,
                );
                RegraBruta::Invalida
            }
        };
        let entao = match entao_expr {
            Some(expr) => self.destino(expr, &format!("o ramo `entao` da decisão `{id}`")),
            None => {
                self.erro(
                    Codigo::ArgumentoAusente,
                    format!("A decisão `{id}` não recebeu `entao`: diga para onde vai quando a regra é verdadeira."),
                    pos_entao,
                );
                DestinoBruto::Invalido
            }
        };
        let senao = match senao_expr {
            Some(expr) => self.destino(expr, &format!("o ramo `senao` da decisão `{id}`")),
            None => {
                self.erro(
                    Codigo::ArgumentoAusente,
                    format!("A decisão `{id}` não recebeu `senao`: toda decisão tem os dois ramos (`senao` é obrigatório)."),
                    pos_senao,
                );
                DestinoBruto::Invalido
            }
        };

        let indice = self.decisoes.len();
        self.idx_decisoes.insert(id.clone(), indice);
        self.decisoes.push(DecisaoColetada {
            id,
            regra,
            entao,
            senao,
            pos,
        });
    }

    fn regra(&mut self, expr: &'a AstExpr, onde: &str) -> RegraBruta {
        let pos = self.pos(expr.span);
        let Some((nome, itens)) = self.como_chamada(expr, &format!("a regra de {onde}")) else {
            return RegraBruta::Invalida;
        };
        match nome.as_str() {
            "igual" | "diferente" | "maior" | "menor" | "contem" => {
                let mut leitor = Leitor::novo(&format!("`{nome}`"), pos, &[], itens);
                let campo = leitor.posicional_texto(self, 0);
                let valor = leitor.posicional_texto(self, 1);
                let pos_campo = leitor.pos_de("__pos0").unwrap_or(pos);
                let pos_valor = leitor.pos_de("__pos1").unwrap_or(pos);
                leitor.fecha(self);
                let sem_campo = campo.is_none();
                let (Some(campo), Some(valor)) = (campo, valor) else {
                    let pos_erro = if sem_campo { pos } else { pos_valor };
                    self.erro(
                        Codigo::CombinadorMalformado,
                        format!("{onde} usa `{nome}` sem o campo e o valor: `{nome}(\"etapa.campo\", \"valor\")`."),
                        pos_erro,
                    );
                    return RegraBruta::Invalida;
                };
                let regra = match nome.as_str() {
                    "igual" => Regra::Igual { campo, valor },
                    "diferente" => Regra::Diferente { campo, valor },
                    "maior" => Regra::Maior { campo, valor },
                    "menor" => Regra::Menor { campo, valor },
                    _ => Regra::Contem { campo, valor },
                };
                RegraBruta::Folha {
                    regra,
                    pos: pos_campo,
                }
            }
            "vazio" => {
                let mut leitor = Leitor::novo("`vazio`", pos, &[], itens);
                let campo = leitor.posicional_texto(self, 0);
                let pos_campo = leitor.pos_de("__pos0").unwrap_or(pos);
                leitor.fecha(self);
                let Some(campo) = campo else {
                    self.erro(
                        Codigo::CombinadorMalformado,
                        format!("{onde} usa `vazio` sem o campo: `vazio(\"etapa.campo\")`."),
                        pos,
                    );
                    return RegraBruta::Invalida;
                };
                RegraBruta::Folha {
                    regra: Regra::Vazio { campo },
                    pos: pos_campo,
                }
            }
            "funcao" => {
                let mut leitor = Leitor::novo("`funcao`", pos, &[], itens);
                let nome_def = leitor.posicional_texto(self, 0);
                leitor.fecha(self);
                let Some(nome_def) = nome_def else {
                    self.erro(
                        Codigo::RegraOuAtorInvalido,
                        format!("{onde} usa `funcao()` sem o nome: `funcao(\"minha_regra\")`."),
                        pos,
                    );
                    return RegraBruta::Invalida;
                };
                RegraBruta::Funcao {
                    nome: nome_def,
                    pos,
                }
            }
            "e" | "ou" | "nao" => {
                let mut posicionais = Vec::new();
                for item in &itens {
                    if item.nome.is_some() {
                        let p = self.pos(item.expr.span);
                        self.erro(
                            Codigo::CombinadorMalformado,
                            format!("{onde} usa `{nome}` com argumento nomeado; `{nome}` recebe os combinadores em sequência."),
                            p,
                        );
                        continue;
                    }
                    posicionais.push(item.expr);
                }
                let valido = if nome == "nao" {
                    posicionais.len() == 1
                } else {
                    posicionais.len() >= 2
                };
                if !valido {
                    let esperado = if nome == "nao" {
                        "exatamente 1 argumento".to_owned()
                    } else {
                        "2 ou mais argumentos".to_owned()
                    };
                    self.erro(
                        Codigo::CombinadorMalformado,
                        format!(
                            "{onde} usa `{nome}` com {} argumento(s); `{nome}` recebe {esperado}.",
                            posicionais.len()
                        ),
                        pos,
                    );
                    return RegraBruta::Invalida;
                }
                let mut filhos = Vec::new();
                for filho in posicionais {
                    filhos.push(self.regra(filho, onde));
                }
                let regra = match nome.as_str() {
                    "e" => Regra::E { args: Vec::new() },
                    "ou" => Regra::Ou { args: Vec::new() },
                    _ => Regra::Nao { args: Vec::new() },
                };
                RegraBruta::Grupo { regra, filhos, pos }
            }
            outro => {
                self.erro(
                    Codigo::RegraOuAtorInvalido,
                    format!(
                        "{onde} tem `se = {outro}(...)`, que não é combinador nem `funcao(\"nome\")`. Use `igual`, `diferente`, `maior`, `menor`, `contem`, `vazio`, `e`, `ou`, `nao` ou `funcao`."
                    ),
                    pos,
                );
                RegraBruta::Invalida
            }
        }
    }

    // ---------------------------------------------------------------- destino e def

    fn destino(&mut self, expr: &'a AstExpr, onde: &str) -> DestinoBruto {
        let pos = self.pos(expr.span);
        match &expr.node {
            Expr::Literal(AstLiteral::String(texto)) => DestinoBruto::Alvo {
                id: texto.node.clone(),
                pos,
            },
            Expr::Call(alvo, args) => {
                let nome = match &alvo.node {
                    Expr::Identifier(ident) => ident.node.ident.clone(),
                    _ => String::new(),
                };
                if nome != "fim" {
                    self.erro(
                        Codigo::TipoErrado,
                        format!("{onde} precisa apontar para o id de uma etapa, de uma decisão ou para `fim(\"id\")`, não para `{nome}(...)`."),
                        pos,
                    );
                    return DestinoBruto::Invalido;
                }
                let itens = self.itens_de(args, "`fim`", pos);
                let mut leitor = Leitor::novo("`fim`", pos, ARGS_FIM, itens);
                let id = leitor.posicional_texto(self, 0);
                let resultado = leitor.texto(self, "resultado");
                let encerra_expr = leitor.item(self, "encerra_atendimento");
                let pos_id = leitor.pos_de("__pos0").unwrap_or(pos);
                leitor.fecha(self);
                let Some(id) = id else {
                    self.erro(
                        Codigo::ArgumentoAusente,
                        format!(
                            "{onde} usa `fim()` sem id: `fim(\"concluido\", resultado = \"...\")`."
                        ),
                        pos,
                    );
                    return DestinoBruto::Invalido;
                };
                if !id_valido(&id) {
                    self.erro(
                        Codigo::IdInvalido,
                        format!("O id do fim `{id}` não é válido: use minúsculas, números e `_`, de {ID_MIN} a {ID_MAX} caracteres."),
                        pos_id,
                    );
                }
                let encerra_atendimento = match encerra_expr {
                    Some(expr) => self
                        .booleano(expr, &format!("`encerra_atendimento` do fim `{id}`"))
                        .unwrap_or(false),
                    None => false,
                };
                DestinoBruto::Fim {
                    fim: Fim {
                        id,
                        resultado,
                        encerra_atendimento,
                    },
                    pos,
                }
            }
            _ => {
                self.erro(
                    Codigo::TipoErrado,
                    format!("{onde} precisa apontar para o id de uma etapa, de uma decisão ou para `fim(\"id\")`, não para {}.", descreve(expr)),
                    pos,
                );
                DestinoBruto::Invalido
            }
        }
    }

    fn coleta_def(&mut self, def: &'a DefP<AstNoPayload>, item: &'a AstStmt, pos: Pos) {
        let nome = def.name.node.ident.clone();
        if nome_permitido(&nome) {
            self.erro(
                Codigo::DefInvalida,
                format!("`{nome}` é nome reservado da linguagem de fluxos; escolha outro nome para a função de regra."),
                pos,
            );
            return;
        }
        if self.pos_defs.contains_key(&nome) {
            self.erro(
                Codigo::DefInvalida,
                format!("Existe mais de uma `def {nome}`: cada função de regra tem nome único."),
                pos,
            );
            return;
        }
        let parametros: Vec<String> = def
            .params
            .iter()
            .filter_map(|parametro| parametro.ident().map(|i| i.node.ident.clone()))
            .collect();
        let so_dados = def.params.len() == 1
            && def
                .params
                .first()
                .map(|parametro| matches!(parametro.node, Parameter::Normal(..)))
                .unwrap_or(false)
            && parametros == vec!["dados".to_owned()];
        if !so_dados {
            let escrito = parametros.join(", ");
            self.erro(
                Codigo::DefInvalida,
                format!("`def {nome}({escrito})` precisa receber um único argumento chamado `dados`: `def {nome}(dados):`."),
                pos,
            );
        }
        let inicio = item.span.begin().get() as usize;
        let fim = (def.body.span.end().get() as usize).max(item.span.end().get() as usize);
        // O fim do trecho cai no começo da declaração seguinte (ou no fim do arquivo), então ele
        // traz junto as linhas em branco que vêm depois da `def`. Espaço em branco não é função:
        // aparar aqui é o que deixa a IR independente da formatação do arquivo, e é o que permite
        // ao formatador (FX-16) reimprimir a `def` sem depender do espaçamento que veio depois
        // dela.
        let fonte = self
            .fonte
            .get(inicio..fim)
            .map(|trecho| trecho.trim_end().to_owned())
            .unwrap_or_default();
        self.pos_defs.insert(nome.clone(), pos);
        self.defs.push(Def { nome, fonte });
    }

    // ---------------------------------------------------------------- leitura de valores

    fn como_chamada(&mut self, expr: &'a AstExpr, onde: &str) -> Option<(String, Vec<Item<'a>>)> {
        match &expr.node {
            Expr::Call(alvo, args) => {
                let nome = match &alvo.node {
                    Expr::Identifier(ident) => ident.node.ident.clone(),
                    _ => {
                        let pos = self.pos(expr.span);
                        self.erro(
                            Codigo::TopoInvalido,
                            format!("{onde} precisa ser a chamada de uma declaração da linguagem, não uma chamada de atributo."),
                            pos,
                        );
                        return None;
                    }
                };
                let pos = self.pos(expr.span);
                let itens = self.itens_de(args, &format!("`{nome}`"), pos);
                Some((nome, itens))
            }
            _ => {
                let pos = self.pos(expr.span);
                self.erro(
                    Codigo::TipoErrado,
                    format!(
                        "{onde} precisa ser uma chamada de declaração, não {}.",
                        descreve(expr)
                    ),
                    pos,
                );
                None
            }
        }
    }

    fn lista_itens(&mut self, expr: &'a AstExpr, onde: &str) -> Vec<&'a AstExpr> {
        match &expr.node {
            Expr::List(itens) => itens.iter().collect(),
            Expr::Tuple(itens) => itens.iter().collect(),
            _ => {
                let pos = self.pos(expr.span);
                self.erro(
                    Codigo::TipoErrado,
                    format!(
                        "{onde} precisa ser uma lista `[...]`, não {}.",
                        descreve(expr)
                    ),
                    pos,
                );
                Vec::new()
            }
        }
    }

    fn lista_texto(&mut self, expr: &'a AstExpr, onde: &str) -> Vec<String> {
        let mut saida = Vec::new();
        for item in self.lista_itens(expr, onde) {
            if let Some(texto) = self.texto(item, onde) {
                saida.push(texto);
            }
        }
        saida
    }

    fn texto(&mut self, expr: &'a AstExpr, onde: &str) -> Option<String> {
        if let Expr::Literal(AstLiteral::String(texto)) = &expr.node {
            return Some(texto.node.clone());
        }
        let pos = self.pos(expr.span);
        let mensagem = if expr_calculada(expr) {
            format!(
                "{onde} precisa ser um valor literal (texto entre aspas): calcule o valor antes de escrever o fluxo."
            )
        } else {
            format!(
                "{onde} precisa ser um texto entre aspas, não {}.",
                descreve(expr)
            )
        };
        self.erro(Codigo::TipoErrado, mensagem, pos);
        None
    }

    fn booleano(&mut self, expr: &'a AstExpr, onde: &str) -> Option<bool> {
        if let Expr::Identifier(ident) = &expr.node {
            match ident.node.ident.as_str() {
                "True" => return Some(true),
                "False" => return Some(false),
                _ => {}
            }
        }
        let pos = self.pos(expr.span);
        self.erro(
            Codigo::TipoErrado,
            format!(
                "{onde} precisa ser `True` ou `False`, não {}.",
                descreve(expr)
            ),
            pos,
        );
        None
    }

    // ---------------------------------------------------------------- passagem 2

    fn fecha(&mut self) -> Option<Ir> {
        self.confere_nomes();

        let Some(fluxo) = self.fluxo.clone() else {
            self.erro(
                Codigo::FluxoAusente,
                "O arquivo não declara `fluxo(...)`. Um arquivo é um fluxo: comece por `fluxo(id = ..., nome = ..., inicio = ..., gatilho = gatilho(...))`.",
                self.pos_fluxo,
            );
            return None;
        };

        let mut inicio = String::new();
        match self.inicio.clone() {
            Some((id, pos)) => {
                if self.idx_etapas.contains_key(&id) {
                    inicio = id;
                } else {
                    self.erro(
                        Codigo::DestinoInexistente,
                        format!("`inicio = \"{id}\"` não é uma etapa declarada. Use o id de uma `etapa(...)` do arquivo."),
                        pos,
                    );
                }
            }
            None => self.erro(
                Codigo::ArgumentoAusente,
                "`fluxo` não recebeu `inicio`: indique o id da etapa em que o atendimento começa.",
                self.pos_fluxo,
            ),
        }

        let mut grafo: BTreeMap<String, Vec<String>> = BTreeMap::new();

        let etapas_coletadas = self.etapas.clone();
        let mut etapas: Vec<Etapa> = Vec::with_capacity(etapas_coletadas.len());
        for coletada in etapas_coletadas {
            let id = coletada.etapa.id.clone();
            let mut etapa = coletada.etapa;
            let mut acoes = Vec::with_capacity(coletada.acoes.len());
            for bruta in coletada.acoes {
                let onde = format!("a ação `{id}.{}`", bruta.acao.id);
                let destino = self.resolve(bruta.destino, &onde);
                if let Some(alvo) = destino.aberto() {
                    grafo.entry(id.clone()).or_default().push(alvo.to_owned());
                }
                let mut acao = bruta.acao;
                acao.vai_para = destino;
                acoes.push(acao);
            }
            etapa.acoes = acoes;
            etapas.push(etapa);
        }

        let decisoes_coletadas = self.decisoes.clone();
        let mut decisoes: Vec<Decisao> = Vec::with_capacity(decisoes_coletadas.len());
        for coletada in &decisoes_coletadas {
            let id = coletada.id.clone();
            let entao = self.resolve(
                clone_bruto(&coletada.entao),
                &format!("o ramo `entao` da decisão `{id}`"),
            );
            let senao = self.resolve(
                clone_bruto(&coletada.senao),
                &format!("o ramo `senao` da decisão `{id}`"),
            );
            for destino in [&entao, &senao] {
                if let Some(alvo) = destino.aberto() {
                    grafo.entry(id.clone()).or_default().push(alvo.to_owned());
                }
            }
            let se = self.fecha_regra(&coletada.regra, &format!("a decisão `{id}`"));
            decisoes.push(Decisao {
                id,
                se,
                entao,
                senao,
            });
        }

        self.confere_ciclo(&grafo);
        self.confere_alcance(&grafo, &inicio, &etapas, &decisoes_coletadas);
        self.confere_uso_dos_campos(&etapas);
        self.confere_uso_das_defs(&decisoes);
        self.confere_efeitos(&etapas);
        self.confere_teto_de_nos();

        if self.tem_erro() {
            return None;
        }

        Some(Ir {
            schema_versao: SCHEMA_VERSAO,
            fluxo,
            gatilho: self.gatilho.clone().unwrap_or(Gatilho {
                quando: String::new(),
                nao_quando: None,
                exemplos: Vec::new(),
            }),
            inicio,
            etapas,
            decisoes,
            fins: self.fins.clone(),
            defs: self.defs.clone(),
        })
    }

    fn resolve(&mut self, bruto: DestinoBruto, onde: &str) -> Destino {
        match bruto {
            DestinoBruto::Alvo { id, pos } => {
                if self.idx_etapas.contains_key(&id) {
                    Destino::Etapa { id }
                } else if self.idx_decisoes.contains_key(&id) {
                    Destino::Decisao { id }
                } else {
                    self.erro(
                        Codigo::DestinoInexistente,
                        format!("{onde} aponta para `{id}`, que não é etapa nem decisão declarada. Use o id de uma `etapa(...)` ou de uma `decisao(...)`."),
                        pos,
                    );
                    Destino::Etapa { id }
                }
            }
            DestinoBruto::Fim { fim, pos } => {
                match self.pos_fins.get(&fim.id).copied() {
                    Some(pos_anterior) => {
                        let anterior = self.fins.iter().find(|f| f.id == fim.id).cloned();
                        if anterior.as_ref() != Some(&fim) {
                            self.erro(
                                Codigo::FimDivergente,
                                format!(
                                    "O fim `{}` aparece com argumentos diferentes aqui e na linha {}: use o mesmo `fim(...)` nos dois lugares.",
                                    fim.id, pos_anterior.linha
                                ),
                                pos,
                            );
                        }
                    }
                    None => {
                        self.pos_fins.insert(fim.id.clone(), pos);
                        self.fins.push(fim.clone());
                    }
                }
                Destino::Fim {
                    id: fim.id,
                    resultado: fim.resultado,
                    encerra_atendimento: fim.encerra_atendimento,
                }
            }
            DestinoBruto::Invalido => Destino::Etapa { id: String::new() },
        }
    }

    fn fecha_regra(&mut self, bruta: &RegraBruta, onde: &str) -> Regra {
        match bruta {
            RegraBruta::Folha { regra, pos } => {
                if let Some(campo) = campo_da_regra(regra) {
                    self.confere_campo_da_regra(&campo, regra, onde, *pos);
                }
                regra.clone()
            }
            RegraBruta::Funcao { nome, pos } => {
                if !self.pos_defs.contains_key(nome) {
                    self.erro(
                        Codigo::RegraOuAtorInvalido,
                        format!("{onde} usa `funcao(\"{nome}\")`, mas não existe `def {nome}(dados)` no arquivo."),
                        *pos,
                    );
                }
                Regra::Funcao { nome: nome.clone() }
            }
            RegraBruta::Grupo { regra, filhos, pos } => {
                self.confere_contradicao(regra, filhos, onde, *pos);
                let mut args = Vec::new();
                for filho in filhos {
                    args.push(self.fecha_regra(filho, onde));
                }
                match regra {
                    Regra::E { .. } => Regra::E { args },
                    Regra::Ou { .. } => Regra::Ou { args },
                    _ => Regra::Nao { args },
                }
            }
            RegraBruta::Invalida => Regra::Vazio {
                campo: String::new(),
            },
        }
    }

    fn confere_campo_da_regra(&mut self, campo: &str, regra: &Regra, onde: &str, pos: Pos) {
        let Some((etapa, nome)) = campo.split_once('.') else {
            self.erro(
                Codigo::CombinadorMalformado,
                format!(
                    "{onde} compara `{campo}`; use `etapa.campo`, por exemplo `coleta.endereco`."
                ),
                pos,
            );
            return;
        };
        if !id_valido(etapa) || !id_valido(nome) {
            self.erro(
                Codigo::CombinadorMalformado,
                format!("{onde} compara `{campo}`; use `etapa.campo`, com ids em minúsculas (por exemplo `coleta.endereco`)."),
                pos,
            );
            return;
        }
        let Some(indice) = self.idx_etapas.get(etapa).copied() else {
            self.erro(
                Codigo::CampoInexistente,
                format!("{onde} compara `{campo}`, mas a etapa `{etapa}` não existe."),
                pos,
            );
            return;
        };
        let campo_info = self.etapas[indice]
            .etapa
            .campos
            .iter()
            .find(|c| c.id == nome)
            .cloned();
        let Some(campo_info) = campo_info else {
            self.erro(
                Codigo::CampoInexistente,
                format!(
                    "{onde} compara `{campo}`, mas a etapa `{etapa}` não tem o campo `{nome}`."
                ),
                pos,
            );
            return;
        };
        if let Some(uso) = self.usos.get_mut(etapa) {
            uso.lidos.insert(nome.to_owned());
        }
        match regra {
            Regra::Igual { valor, .. } | Regra::Diferente { valor, .. } => {
                if !campo_info.opcoes.is_empty() && !campo_info.opcoes.iter().any(|o| o == valor) {
                    self.aviso(
                        Codigo::ForaDasOpcoes,
                        format!(
                            "{onde} compara `{campo}` com `{valor}`, que não é uma das opções ({}): o teste nunca casa.",
                            campo_info.opcoes.join(", ")
                        ),
                        pos,
                    );
                }
            }
            Regra::Maior { .. } | Regra::Menor { .. } if campo_info.tipo != FieldKind::Number => {
                self.aviso(
                    Codigo::ComparacaoDeTexto,
                    format!(
                        "{onde} usa `maior`/`menor` com `{campo}`, mas o campo é `{}`: mude o tipo para `numero` ou use `igual`.",
                        campo_info.tipo.as_text()
                    ),
                    pos,
                );
            }
            _ => {}
        }
    }

    fn confere_contradicao(&mut self, regra: &Regra, filhos: &[RegraBruta], onde: &str, pos: Pos) {
        let folhas: Vec<&Regra> = filhos
            .iter()
            .filter_map(|filho| match filho {
                RegraBruta::Folha { regra, .. } => Some(regra),
                _ => None,
            })
            .collect();
        let eh_e = matches!(regra, Regra::E { .. });
        let eh_ou = matches!(regra, Regra::Ou { .. });
        let mut avisos: Vec<String> = Vec::new();
        for (i, a) in folhas.iter().enumerate() {
            for b in folhas.iter().skip(i + 1) {
                let (campo_a, valor_a, tipo_a) = folha_chave(a);
                let (campo_b, valor_b, tipo_b) = folha_chave(b);
                if eh_e
                    && campo_a.is_some()
                    && campo_a == campo_b
                    && valor_a == valor_b
                    && matches!(
                        (tipo_a, tipo_b),
                        ("igual", "diferente") | ("diferente", "igual")
                    )
                {
                    avisos.push(format!(
                        "{onde} não pode ser verdadeira com `{campo}` igual e diferente de `{valor}` ao mesmo tempo.",
                        campo = campo_a.unwrap_or(""),
                        valor = valor_a.unwrap_or("")
                    ));
                }
                if eh_ou && a == b {
                    avisos.push(format!(
                        "{onde} repete o mesmo teste duas vezes na mesma disjunção (`ou`)."
                    ));
                }
            }
        }
        for mensagem in avisos {
            self.aviso(Codigo::CondicaoContraditoria, mensagem, pos);
        }
    }

    fn confere_ciclo(&mut self, grafo: &BTreeMap<String, Vec<String>>) {
        let mut estado: BTreeMap<String, u8> = BTreeMap::new();
        let mut caminho: Vec<String> = Vec::new();
        let mut ciclo: Option<Vec<String>> = None;
        let raizes: Vec<String> = self.idx_decisoes.keys().cloned().collect();
        for raiz in &raizes {
            if ciclo.is_some() {
                break;
            }
            self.visita_ciclo(raiz, grafo, &mut estado, &mut caminho, &mut ciclo);
        }
        let Some(ciclo) = ciclo else { return };
        let pos = raizes
            .iter()
            .position(|r| Some(r) == ciclo.first())
            .and_then(|_| {
                self.idx_decisoes
                    .get(&ciclo[0])
                    .and_then(|i| self.decisoes.get(*i))
                    .map(|d| d.pos)
            })
            .unwrap_or(self.pos_fluxo);
        self.erro(
            Codigo::CicloDeDecisao,
            format!(
                "As decisões formam um ciclo ({}): um fluxo não pode voltar a uma decisão já avaliada.",
                ciclo.join(" → ")
            ),
            pos,
        );
    }

    fn visita_ciclo(
        &self,
        no: &str,
        grafo: &BTreeMap<String, Vec<String>>,
        estado: &mut BTreeMap<String, u8>,
        caminho: &mut Vec<String>,
        ciclo: &mut Option<Vec<String>>,
    ) {
        if ciclo.is_some() || !self.idx_decisoes.contains_key(no) {
            return;
        }
        match estado.get(no).copied().unwrap_or(0) {
            1 => {
                let mut encontrado: Vec<String> = caminho
                    .iter()
                    .skip_while(|atual| atual.as_str() != no)
                    .cloned()
                    .collect();
                encontrado.push(no.to_owned());
                *ciclo = Some(encontrado);
                return;
            }
            2 => return,
            _ => {}
        }
        estado.insert(no.to_owned(), 1);
        caminho.push(no.to_owned());
        if let Some(destinos) = grafo.get(no) {
            for destino in destinos {
                self.visita_ciclo(destino, grafo, estado, caminho, ciclo);
            }
        }
        caminho.pop();
        estado.insert(no.to_owned(), 2);
    }

    fn confere_alcance(
        &mut self,
        grafo: &BTreeMap<String, Vec<String>>,
        inicio: &str,
        etapas: &[Etapa],
        decisoes: &[DecisaoColetada],
    ) {
        let mut alcancados: BTreeSet<String> = BTreeSet::new();
        if !inicio.is_empty() {
            let mut pilha = vec![inicio.to_owned()];
            while let Some(no) = pilha.pop() {
                if no.is_empty() || !alcancados.insert(no.clone()) {
                    continue;
                }
                if let Some(destinos) = grafo.get(&no) {
                    for destino in destinos {
                        pilha.push(destino.clone());
                    }
                }
            }
        }
        let mut posicoes: BTreeMap<String, Pos> = BTreeMap::new();
        for etapa in &self.etapas {
            posicoes.insert(etapa.etapa.id.clone(), etapa.pos);
        }
        let mut avisos: Vec<(Codigo, String, Pos)> = Vec::new();
        for etapa in etapas {
            let pos = posicoes.get(&etapa.id).copied().unwrap_or(self.pos_fluxo);
            if etapa.acoes.is_empty() {
                avisos.push((
                    Codigo::EtapaSemAcao,
                    format!(
                        "A etapa `{}` não tem nenhuma `acao(...)`: o atendimento não sai dela. Acrescente uma ação com `vai_para`.",
                        etapa.id
                    ),
                    pos,
                ));
            }
            if !inicio.is_empty() && !alcancados.contains(&etapa.id) {
                avisos.push((
                    Codigo::Inalcancavel,
                    format!(
                        "A etapa `{}` não é alcançável a partir de `inicio = \"{inicio}\"`. Confira os `vai_para`.",
                        etapa.id
                    ),
                    pos,
                ));
            }
        }
        for decisao in decisoes {
            if !inicio.is_empty() && !alcancados.contains(&decisao.id) {
                avisos.push((
                    Codigo::Inalcancavel,
                    format!(
                        "A decisão `{}` não é alcançável a partir de `inicio = \"{inicio}\"`.",
                        decisao.id
                    ),
                    decisao.pos,
                ));
            }
        }
        for (codigo, mensagem, pos) in avisos {
            self.aviso(codigo, mensagem, pos);
        }
    }

    fn confere_uso_dos_campos(&mut self, etapas: &[Etapa]) {
        let mut avisos: Vec<(String, Pos)> = Vec::new();
        for etapa in etapas {
            let usos = self.usos.get(&etapa.id).cloned().unwrap_or_default();
            let pos_etapa = self
                .idx_etapas
                .get(&etapa.id)
                .and_then(|i| self.etapas.get(*i))
                .map(|e| e.pos)
                .unwrap_or(self.pos_fluxo);
            let posicoes: BTreeMap<String, Pos> = self
                .idx_etapas
                .get(&etapa.id)
                .and_then(|i| self.etapas.get(*i))
                .map(|e| e.pos_campos.clone())
                .unwrap_or_default();
            for campo in &etapa.campos {
                if usos.exigidos.contains(&campo.id) || usos.lidos.contains(&campo.id) {
                    continue;
                }
                let pos = posicoes.get(&campo.id).copied().unwrap_or(pos_etapa);
                avisos.push((
                    format!(
                        "O campo `{}.{}` não é exigido por nenhuma ação nem lido por nenhuma regra: ninguém usa o valor.",
                        etapa.id, campo.id
                    ),
                    pos,
                ));
            }
        }
        for (mensagem, pos) in avisos {
            self.aviso(Codigo::CampoNaoUsado, mensagem, pos);
        }
    }

    fn confere_uso_das_defs(&mut self, decisoes: &[Decisao]) {
        let mut usadas: BTreeSet<String> = BTreeSet::new();
        for decisao in decisoes {
            coleta_funcoes(&decisao.se, &mut usadas);
        }
        let defs: Vec<(String, Pos)> = self
            .defs
            .iter()
            .map(|def| {
                let pos = self
                    .pos_defs
                    .get(&def.nome)
                    .copied()
                    .unwrap_or(self.pos_fluxo);
                (def.nome.clone(), pos)
            })
            .collect();
        for (nome, pos) in defs {
            if usadas.contains(&nome) {
                continue;
            }
            self.aviso(
                Codigo::DefNaoUsada,
                format!(
                    "A função `{nome}` está definida e nenhuma decisão usa `funcao(\"{nome}\")`. Chame ou apague."
                ),
                pos,
            );
        }
    }

    fn confere_efeitos(&mut self, etapas: &[Etapa]) {
        let mut campos: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for etapa in etapas {
            campos.insert(
                etapa.id.as_str(),
                etapa.campos.iter().map(|c| c.id.as_str()).collect(),
            );
        }
        let pendentes: Vec<(String, String, Pos, String, bool)> = self
            .efeitos
            .iter()
            .filter_map(|coletado| {
                let (texto, o_que) = match &coletado.efeito {
                    Efeito::MensagemAoCliente { texto } => (texto.clone(), "mensagem ao cliente"),
                    Efeito::NotificarWhatsapp { mensagem, .. } => {
                        (mensagem.clone(), "notificação para a equipe")
                    }
                    Efeito::Encaminhar { .. } => return None,
                };
                Some((
                    texto,
                    o_que.to_owned(),
                    coletado.pos_texto,
                    coletado.etapa.clone(),
                    coletado.ao_entrar,
                ))
            })
            .collect();
        let mut avisos: Vec<(Codigo, String, Pos)> = Vec::new();
        for (texto, o_que, pos, etapa_do_efeito, ao_entrar) in pendentes {
            for variavel in variaveis(&texto) {
                let Some((nome_etapa, nome_campo)) = variavel.split_once('.') else {
                    continue;
                };
                if variavel == "ficha.id" || variavel == "cliente.peer" {
                    continue;
                }
                let existe = campos
                    .get(nome_etapa)
                    .map(|campos| campos.contains(nome_campo))
                    .unwrap_or(false);
                if !existe {
                    avisos.push((
                        Codigo::VariavelSemCampo,
                        format!(
                            "A {o_que} usa `{{{variavel}}}`, mas a etapa `{nome_etapa}` não tem o campo `{nome_campo}`: no atendimento sai vazio."
                        ),
                        pos,
                    ));
                    continue;
                }
                if ao_entrar && nome_etapa == etapa_do_efeito {
                    avisos.push((
                        Codigo::VariavelDoProprioPasso,
                        format!(
                            "`ao_entrar` da etapa `{etapa_do_efeito}` usa `{{{variavel}}}`, que só é preenchido dentro da própria etapa: a mensagem sai com o texto vazio."
                        ),
                        pos,
                    ));
                }
            }
        }
        for (codigo, mensagem, pos) in avisos {
            self.aviso(codigo, mensagem, pos);
        }
    }

    fn confere_teto_de_nos(&mut self) {
        let nos = self.etapas.len() + self.idx_decisoes.len() + self.fins.len();
        if nos > MAX_NOS {
            let pos = self.pos_fluxo;
            self.erro(
                Codigo::NosDemais,
                format!("O fluxo tem {nos} nós e o limite é {MAX_NOS}. Junte etapas parecidas ou divida o fluxo."),
                pos,
            );
        }
    }

    // ---------------------------------------------------------------- nomes e variáveis

    fn confere_nomes(&mut self) {
        let locais = BTreeSet::new();
        self.confere_nomes_stmt(self.modulo.statement(), &locais);
    }

    fn confere_nomes_stmt(&mut self, stmt: &'a AstStmt, locais: &BTreeSet<String>) {
        match &stmt.node {
            Stmt::Statements(itens) => {
                for item in itens {
                    self.confere_nomes_stmt(item, locais);
                }
            }
            Stmt::Def(def) => {
                let mut escopo = locais.clone();
                for parametro in &def.params {
                    if let Some(ident) = parametro.ident() {
                        escopo.insert(ident.node.ident.clone());
                    }
                }
                coleta_assigns_stmt(&def.body, &mut escopo);
                self.confere_nomes_stmt(&def.body, &escopo);
            }
            Stmt::Expression(expr) => self.confere_nomes_expr(expr, locais),
            Stmt::Assign(assign) => self.confere_nomes_expr(&assign.rhs, locais),
            Stmt::AssignModify(_, _, expr) => self.confere_nomes_expr(expr, locais),
            Stmt::Return(Some(expr)) => self.confere_nomes_expr(expr, locais),
            Stmt::Return(None) | Stmt::Break | Stmt::Continue | Stmt::Pass | Stmt::Load(_) => {}
            Stmt::If(cond, corpo) => {
                self.confere_nomes_expr(cond, locais);
                self.confere_nomes_stmt(corpo, locais);
            }
            Stmt::IfElse(cond, ramos) => {
                self.confere_nomes_expr(cond, locais);
                self.confere_nomes_stmt(&ramos.0, locais);
                self.confere_nomes_stmt(&ramos.1, locais);
            }
            Stmt::For(para) => {
                self.confere_nomes_expr(&para.over, locais);
                self.confere_nomes_stmt(&para.body, locais);
            }
        }
    }

    fn confere_nomes_expr(&mut self, expr: &'a AstExpr, locais: &BTreeSet<String>) {
        match &expr.node {
            Expr::Identifier(ident) => self.confere_nome(ident, locais),
            Expr::Tuple(itens) | Expr::List(itens) => {
                for item in itens {
                    self.confere_nomes_expr(item, locais);
                }
            }
            Expr::Dict(pares) => {
                for (chave, valor) in pares {
                    self.confere_nomes_expr(chave, locais);
                    self.confere_nomes_expr(valor, locais);
                }
            }
            Expr::Dot(alvo, _) => self.confere_nomes_expr(alvo, locais),
            Expr::Call(alvo, args) => {
                self.confere_nomes_expr(alvo, locais);
                for arg in &args.args {
                    self.confere_nomes_expr(arg.expr(), locais);
                }
            }
            Expr::Index(par) => {
                self.confere_nomes_expr(&par.0, locais);
                self.confere_nomes_expr(&par.1, locais);
            }
            Expr::Index2(par) => {
                self.confere_nomes_expr(&par.0, locais);
                self.confere_nomes_expr(&par.1, locais);
                self.confere_nomes_expr(&par.2, locais);
            }
            Expr::Slice(alvo, a, b, c) => {
                self.confere_nomes_expr(alvo, locais);
                for parte in [a, b, c].into_iter().flatten() {
                    self.confere_nomes_expr(parte, locais);
                }
            }
            Expr::Lambda(lambda) => self.confere_nomes_expr(&lambda.body, locais),
            Expr::Not(expr) | Expr::Minus(expr) | Expr::Plus(expr) | Expr::BitNot(expr) => {
                self.confere_nomes_expr(expr, locais);
            }
            Expr::Op(a, _, b) => {
                self.confere_nomes_expr(a, locais);
                self.confere_nomes_expr(b, locais);
            }
            Expr::If(trio) => {
                self.confere_nomes_expr(&trio.0, locais);
                self.confere_nomes_expr(&trio.1, locais);
                self.confere_nomes_expr(&trio.2, locais);
            }
            Expr::ListComprehension(corpo, para, clausulas) => {
                let mut escopo = locais.clone();
                coleta_assigns_target(&para.var, &mut escopo);
                self.confere_nomes_expr(&para.over, &escopo);
                for clausula in clausulas {
                    match clausula {
                        Clause::For(para) => {
                            coleta_assigns_target(&para.var, &mut escopo);
                            self.confere_nomes_expr(&para.over, &escopo);
                        }
                        Clause::If(cond) => self.confere_nomes_expr(cond, &escopo),
                    }
                }
                self.confere_nomes_expr(corpo, &escopo);
            }
            Expr::DictComprehension(par, para, clausulas) => {
                let mut escopo = locais.clone();
                coleta_assigns_target(&para.var, &mut escopo);
                self.confere_nomes_expr(&para.over, &escopo);
                for clausula in clausulas {
                    match clausula {
                        Clause::For(para) => {
                            coleta_assigns_target(&para.var, &mut escopo);
                            self.confere_nomes_expr(&para.over, &escopo);
                        }
                        Clause::If(cond) => self.confere_nomes_expr(cond, &escopo),
                    }
                }
                self.confere_nomes_expr(&par.0, &escopo);
                self.confere_nomes_expr(&par.1, &escopo);
            }
            Expr::FString(f) => {
                for parte in &f.expressions {
                    self.confere_nomes_expr(parte, locais);
                }
            }
            Expr::Literal(_) => {}
        }
    }

    fn confere_nome(&mut self, ident: &'a AstIdent, locais: &BTreeSet<String>) {
        let nome = ident.node.ident.as_str();
        if nome_permitido(nome) || locais.contains(nome) {
            return;
        }
        let pos = self.pos(ident.span);
        self.erro(
            Codigo::NomeInexistente,
            format!(
                "`{nome}` não existe na linguagem de fluxos: o script não tem relógio, rede, arquivo nem aleatório. Use as etapas, os campos e os combinadores do fluxo."
            ),
            pos,
        );
    }

    fn confere_formato_das_variaveis(&mut self, texto: &str, onde: &str, pos: Pos) {
        for variavel in variaveis(texto) {
            let valida = variavel == "ficha.id"
                || variavel == "cliente.peer"
                || variavel
                    .split_once('.')
                    .map(|(etapa, campo)| {
                        etapa != "ficha"
                            && etapa != "cliente"
                            && id_valido(etapa)
                            && id_valido(campo)
                    })
                    .unwrap_or(false);
            if valida {
                continue;
            }
            self.erro(
                Codigo::VariavelInvalida,
                format!(
                    "{onde} usa `{{{variavel}}}`, que não existe. As variáveis são `{{etapa.campo}}`, `{{ficha.id}}` e `{{cliente.peer}}`."
                ),
                pos,
            );
        }
    }
}

/// Efeito lido da árvore, ainda sem saber de qual etapa veio.
struct EfeitoLido {
    efeito: Efeito,
    pos_texto: Pos,
}

// ------------------------------------------------------------------ auxiliares

fn id_valido(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(primeiro) if primeiro.is_ascii_lowercase() => {}
        _ => return false,
    }
    id.len() >= ID_MIN
        && id.len() <= ID_MAX
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Quais variáveis `{...}` aparecem no texto, na ordem.
pub(crate) fn variaveis(texto: &str) -> Vec<String> {
    let mut saida = Vec::new();
    let mut resto = texto;
    while let Some(inicio) = resto.find('{') {
        let depois = &resto[inicio + 1..];
        match depois.find('}') {
            Some(fim) => {
                saida.push(depois[..fim].to_owned());
                resto = &depois[fim + 1..];
            }
            None => break,
        }
    }
    saida
}

fn descreve(expr: &AstExpr) -> String {
    match &expr.node {
        Expr::Literal(AstLiteral::String(_)) => "um texto".to_owned(),
        Expr::Literal(AstLiteral::Int(_)) => "um número inteiro".to_owned(),
        Expr::Literal(AstLiteral::Float(_)) => "um número".to_owned(),
        Expr::Literal(_) => "um valor literal".to_owned(),
        Expr::List(_) => "uma lista".to_owned(),
        Expr::Tuple(_) => "uma tupla".to_owned(),
        Expr::Dict(_) => "um dicionário".to_owned(),
        Expr::Identifier(ident) => match ident.node.ident.as_str() {
            "True" | "False" => "um booleano".to_owned(),
            "None" => "`None`".to_owned(),
            outro => format!("o nome `{outro}`"),
        },
        Expr::Call(alvo, _) => match &alvo.node {
            Expr::Identifier(ident) => format!("a chamada `{}(...)`", ident.node.ident),
            _ => "uma chamada".to_owned(),
        },
        Expr::FString(_) => "uma f-string".to_owned(),
        _ => "um valor calculado".to_owned(),
    }
}

fn expr_calculada(expr: &AstExpr) -> bool {
    matches!(
        expr.node,
        Expr::Op(..) | Expr::FString(_) | Expr::Index(..) | Expr::Lambda(_)
    )
}

fn campo_da_regra(regra: &Regra) -> Option<String> {
    match regra {
        Regra::Igual { campo, .. }
        | Regra::Diferente { campo, .. }
        | Regra::Maior { campo, .. }
        | Regra::Menor { campo, .. }
        | Regra::Contem { campo, .. }
        | Regra::Vazio { campo } => Some(campo.clone()),
        _ => None,
    }
}

fn clone_bruto(bruto: &DestinoBruto) -> DestinoBruto {
    match bruto {
        DestinoBruto::Alvo { id, pos } => DestinoBruto::Alvo {
            id: id.clone(),
            pos: *pos,
        },
        DestinoBruto::Fim { fim, pos } => DestinoBruto::Fim {
            fim: fim.clone(),
            pos: *pos,
        },
        DestinoBruto::Invalido => DestinoBruto::Invalido,
    }
}

fn folha_chave(regra: &Regra) -> (Option<&str>, Option<&str>, &'static str) {
    match regra {
        Regra::Igual { campo, valor } => (Some(campo), Some(valor), "igual"),
        Regra::Diferente { campo, valor } => (Some(campo), Some(valor), "diferente"),
        Regra::Maior { campo, valor } => (Some(campo), Some(valor), "maior"),
        Regra::Menor { campo, valor } => (Some(campo), Some(valor), "menor"),
        Regra::Contem { campo, valor } => (Some(campo), Some(valor), "contem"),
        Regra::Vazio { campo } => (Some(campo), None, "vazio"),
        _ => (None, None, ""),
    }
}

fn coleta_funcoes(regra: &Regra, saida: &mut BTreeSet<String>) {
    match regra {
        Regra::Funcao { nome } => {
            saida.insert(nome.clone());
        }
        Regra::E { args } | Regra::Ou { args } | Regra::Nao { args } => {
            for filho in args {
                coleta_funcoes(filho, saida);
            }
        }
        _ => {}
    }
}

fn coleta_assigns_stmt(stmt: &AstStmt, escopo: &mut BTreeSet<String>) {
    match &stmt.node {
        Stmt::Statements(itens) => {
            for item in itens {
                coleta_assigns_stmt(item, escopo);
            }
        }
        Stmt::Assign(assign) => coleta_assigns_target(&assign.lhs, escopo),
        Stmt::AssignModify(alvo, _, _) => coleta_assigns_target(alvo, escopo),
        Stmt::If(_, corpo) => coleta_assigns_stmt(corpo, escopo),
        Stmt::IfElse(_, ramos) => {
            coleta_assigns_stmt(&ramos.0, escopo);
            coleta_assigns_stmt(&ramos.1, escopo);
        }
        Stmt::For(para) => {
            coleta_assigns_target(&para.var, escopo);
            coleta_assigns_stmt(&para.body, escopo);
        }
        Stmt::Def(def) => {
            escopo.insert(def.name.node.ident.clone());
        }
        _ => {}
    }
}

fn coleta_assigns_target(alvo: &AstAssignTarget, escopo: &mut BTreeSet<String>) {
    use starlark::syntax::ast::AssignTarget;
    match &alvo.node {
        AssignTarget::Identifier(ident) => {
            escopo.insert(ident.node.ident.clone());
        }
        AssignTarget::Tuple(itens) => {
            for item in itens {
                coleta_assigns_target(item, escopo);
            }
        }
        AssignTarget::Index(_) | AssignTarget::Dot(..) => {}
    }
}

/// Leitor de argumentos de uma chamada: consome por nome ou por posição e cobra o que sobrar.
struct Leitor<'a> {
    onde: String,
    conhecidos: &'static [&'static str],
    itens: Vec<Item<'a>>,
    usados: Vec<bool>,
    posicoes: BTreeMap<String, Pos>,
}

impl<'a> Leitor<'a> {
    fn novo(
        onde: &str,
        _pos: Pos,
        conhecidos: &'static [&'static str],
        itens: Vec<Item<'a>>,
    ) -> Self {
        let usados = vec![false; itens.len()];
        Self {
            onde: onde.to_owned(),
            conhecidos,
            itens,
            usados,
            posicoes: BTreeMap::new(),
        }
    }

    fn item(&mut self, forma: &mut Forma<'a>, nome: &str) -> Option<&'a AstExpr> {
        for (i, item) in self.itens.iter().enumerate() {
            if item.nome.as_deref() == Some(nome) && !self.usados[i] {
                self.usados[i] = true;
                let pos = forma.pos(item.expr.span);
                self.posicoes.insert(nome.to_owned(), pos);
                return Some(item.expr);
            }
        }
        None
    }

    /// O `indice`-ésimo argumento posicional (contando todos, e não só os que sobraram).
    fn posicional(&mut self, indice: usize) -> Option<&'a AstExpr> {
        let mut visto = 0usize;
        for (i, item) in self.itens.iter().enumerate() {
            if item.nome.is_some() {
                continue;
            }
            if visto == indice {
                if self.usados[i] {
                    return None;
                }
                self.usados[i] = true;
                return Some(item.expr);
            }
            visto += 1;
        }
        None
    }

    fn pos_de(&self, chave: &str) -> Option<Pos> {
        self.posicoes.get(chave).copied()
    }

    fn texto(&mut self, forma: &mut Forma<'a>, nome: &str) -> Option<String> {
        let expr = self.item(forma, nome)?;
        let onde = format!("o argumento `{nome}` de {}", self.onde);
        forma.texto(expr, &onde)
    }

    fn posicional_texto(&mut self, forma: &mut Forma<'a>, indice: usize) -> Option<String> {
        let expr = self.posicional(indice)?;
        let pos = forma.pos(expr.span);
        self.posicoes.insert(format!("__pos{indice}"), pos);
        let onde = format!("o argumento posicional {} de {}", indice + 1, self.onde);
        forma.texto(expr, &onde)
    }

    /// Cobra o que sobrar: argumento nomeado desconhecido ou posicional a mais.
    fn fecha(&mut self, forma: &mut Forma<'a>) {
        let mut sobras: Vec<(Option<String>, Pos)> = Vec::new();
        for (i, item) in self.itens.iter().enumerate() {
            if self.usados.get(i).copied().unwrap_or(false) {
                continue;
            }
            let pos = forma.pos(item.expr.span);
            sobras.push((item.nome.clone(), pos));
        }
        let conhecidos = if self.conhecidos.is_empty() {
            String::new()
        } else {
            format!(" Os argumentos são: {}.", self.conhecidos.join(", "))
        };
        let onde = self.onde.clone();
        for (nome, pos) in sobras {
            let mensagem = match nome {
                Some(nome) => format!(
                    "{onde} não tem o argumento `{nome}`.{conhecidos} Confira a escrita: argumento com nome errado é recusado em vez de ignorado."
                ),
                None => format!("{onde} não aceita esse argumento posicional a mais.{conhecidos}"),
            };
            forma.erro(Codigo::ArgumentoDesconhecido, mensagem, pos);
        }
    }
}
