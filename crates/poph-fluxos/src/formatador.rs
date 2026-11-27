//! Formatador: forma canônica do script de fluxo (script ⇄ estrutura) — FX-16.
//!
//! Duas funções públicas, e as duas são **puras** (sem banco, relógio, rede ou arquivo):
//!
//! - [`formatar`] — recebe o texto de um fluxo e devolve o **mesmo fluxo na forma canônica**. Compila
//!   o texto com o compilador da FX-10 e reimprime a estrutura; script com erro devolve
//!   [`FalhaDeFormato`] com os diagnósticos e **nunca** texto parcial. Formatar duas vezes dá o
//!   mesmo texto.
//! - [`imprimir`] — recebe a IR e gera o script. É o que permite à UI futura editar a estrutura e
//!   gravar script: a ordem e o estilo são fixos, então a mesma IR dá sempre o mesmo texto.
//!
//! ## A forma canônica
//!
//! Uma declaração por bloco, na ordem **`fluxo`, `def`, `etapa`, `decisao`**, separados por uma
//! linha em branco e com uma linha em branco só no fim do arquivo. A ordem das `etapa` e das
//! `decisao` é a da IR (a do script de origem), e as `def` vão **antes de quem as usa** — o
//! `funcao("nome")` de uma decisão e as chamadas entre `def` só acham a função se ela estiver
//! definida acima.
//!
//! Dentro de uma declaração de topo, um argumento por linha com recuo de 4 espaços. Chamada
//! aninhada cabe em uma linha quando cabe em [`LARGURA`] colunas; quando não cabe, abre em várias
//! linhas. Três exceções, de propósito:
//!
//! - `campos`, `acoes` e `ao_entrar` de uma etapa ficam **sempre** abertos: são a estrutura da
//!   etapa, um item por linha, e é o que deixa o diff de revisão legível. `efeitos` de uma ação não
//!   segue essa regra — é detalhe da ação e cabe na linha quando é curto.
//! - Os combinadores compostos (`e`, `ou`, `nao`) também ficam sempre abertos: uma regra composta
//!   se lê como lista.
//! - Texto nunca é partido: não há como quebrar um literal sem mudar o valor.
//!
//! Os argumentos opcionais que estão no padrão não aparecem: sem `atores` a etapa é `["agente"]`,
//! sem `obrigatorio` o campo é opcional, sem `opcoes` o campo não é `opcao`, lista vazia não é
//! escrita. Os três exemplos do repositório (`tests/fluxos/exemplos/*.star`) cabem em [`LARGURA`] e
//! voltam iguais quando já não têm `def` fora de lugar.
//!
//! ## Comentário
//!
//! A IR não guarda comentário (a única exceção é o trecho da `def`, guardado literalmente em
//! [`crate::ir::Def::fonte`]). Por isso a regra, decidida na especificação da tarefa:
//!
//! - **Comentário ligado a uma declaração é preservado**: o bloco de linhas `#` cuja última linha é
//!   imediatamente a anterior da linha da declaração de topo. O bloco é reimpresso alinhado à
//!   esquerda, acima da declaração.
//! - **Comentário solto é descartado**: dentro de uma declaração, no fim de uma linha, no fim do
//!   arquivo, ou separado da declaração por uma linha em branco. Formatar de novo não o traz de
//!   volta, então o texto canônico não muda depois da primeira passagem.
//! - **Comentário dentro da `def`** vem de graça: o trecho guardado na IR é impresso literalmente.
//!
//! ## Limite conhecido
//!
//! A ordem dos `fim(...)` na IR é a da **primeira aparição** no script. A forma canônica agrupa
//! `etapa` antes de `decisao`, então um script que declara uma `decisao` com `fim(...)` **antes** de
//! uma `etapa` que declara outro `fim(...)` reordena os dois na IR recompilada. O fluxo é o mesmo
//! (`inicio`, etapas, decisões e fins iguais no conteúdo); muda só a ordem da lista `fins`, e é o
//! único caso em que `carregar` (FX-10) acusaria divergência depois de formatar. Não há como evitar
//! sem uma IR que guarde a posição de cada declaração, o que é mudança de contrato.

use std::collections::BTreeMap;
use std::fmt;

use poph_types::ActionActor;
use starlark::syntax::ast::{Argument, AstLiteral, AstNoPayload, AstStmt, CallArgsP, Expr, Stmt};

use crate::compilador::{Limites, compile};
use crate::diagnostics::Diagnostic;
use crate::ir::{Acao, Campo, Decisao, Destination, Effect, Etapa, Gatilho, Ir, Rule};
use crate::language::parse_ast;

/// Largura máxima, em colunas, de uma linha da forma canônica.
///
/// É o maior comprimento de linha dos três exemplos do repositório (`112`), então formatar um deles
/// não muda nada. Linha mais larga que isto só acontece com um texto (literal) que sozinho não cabe.
pub const LARGURA: usize = 112;

/// Recuo de um nível de aninhamento.
const RECUO: usize = 4;

/// O script não compila: os diagnósticos dizem o que corrigir.
///
/// A `formatar` devolve isto em vez de texto parcial — não existe "formatou até onde deu".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalhaDeFormato {
    /// Todos os diagnósticos do script (erros e avisos), na ordem em que o compilador os achou.
    pub diagnosticos: Vec<Diagnostic>,
}

impl FalhaDeFormato {
    /// Código estável da falha, para log e API.
    pub fn codigo(&self) -> &'static str {
        "fluxo_invalido"
    }

    /// Só os erros, na ordem do compilador.
    pub fn erros(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnosticos.iter().filter(|d| d.is_error())
    }
}

impl fmt::Display for FalhaDeFormato {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut erros = self.erros();
        match erros.next() {
            Some(primeiro) => {
                f.write_str("- ")?;
                if let (Some(linha), Some(coluna)) = (primeiro.line, primeiro.column) {
                    write!(f, "{linha}:{coluna} ")?;
                }
                f.write_str(&primeiro.mensagem)
            }
            None => f.write_str("o script não compila"),
        }
    }
}

impl std::error::Error for FalhaDeFormato {}

/// Devolve o script na forma canônica.
///
/// Compila o texto (FX-10) e reimprime a estrutura. Comentário ligado a declaração é preservado;
/// comentário solto é descartado (ver o cabeçalho do módulo).
///
/// # Erros
///
/// Script que não compila devolve [`FalhaDeFormato`] — sintaxe, forma, limite ou estrutura — e
/// **nenhum texto**. Aviso não impede formatar.
pub fn formatar(fonte: &str) -> Result<String, FalhaDeFormato> {
    formatar_com(fonte, &Limites::default())
}

/// Como [`formatar`], com os limites do processo (`fluxos.max_fonte_bytes`, `fluxos.max_nos`), que
/// quem chama do servidor lê com [`Limites::from_config`] (FX-26b).
///
/// # Erros
///
/// O mesmo de [`formatar`].
pub fn formatar_com(fonte: &str, limites: &Limites) -> Result<String, FalhaDeFormato> {
    let compilado = compile(fonte, limites);
    let Some(ir) = compilado.ir else {
        return Err(FalhaDeFormato {
            diagnosticos: compilado.diagnostics,
        });
    };
    Ok(montar(&ir, &comentarios(fonte)))
}

/// Gera o script de fluxo a partir da IR, na forma canônica.
///
/// Determinístico: a mesma IR dá byte a byte o mesmo texto. O trecho de cada `def` é impresso
/// **literalmente** (é o que a IR guarda em [`crate::ir::Def::fonte`]); uma IR escrita à mão com
/// `fonte` vazia não produz script válido — não há como reconstruir o corpo de uma função.
pub fn imprimir(ir: &Ir) -> String {
    montar(ir, &BTreeMap::new())
}

// ------------------------------------------------------------------ montagem

/// Identidade de uma declaração de topo, para ligar o comentário à declaração certa.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Chave {
    /// O `fluxo(...)`.
    Fluxo,
    /// A `def` com este nome.
    Funcao(String),
    /// A `etapa` com este id.
    Etapa(String),
    /// A `decisao` com este id.
    Decisao(String),
}

/// Monta o texto: uma declaração por bloco, separadas por linha em branco, terminando em `\n`.
fn montar(ir: &Ir, comentarios: &BTreeMap<Chave, Vec<String>>) -> String {
    let mut partes: Vec<String> = Vec::with_capacity(ir.stages.len() + ir.decisions.len() + 2);
    partes.push(bloco(Chave::Fluxo, corpo(&valor_do_fluxo(ir)), comentarios));
    for definicao in &ir.defs {
        partes.push(bloco(
            Chave::Funcao(definicao.nome.clone()),
            definicao.fonte.clone(),
            comentarios,
        ));
    }
    for etapa in &ir.stages {
        partes.push(bloco(
            Chave::Etapa(etapa.id.clone()),
            corpo(&valor_da_etapa(etapa)),
            comentarios,
        ));
    }
    for decisao in &ir.decisions {
        partes.push(bloco(
            Chave::Decisao(decisao.id.clone()),
            corpo(&valor_da_decisao(decisao)),
            comentarios,
        ));
    }
    let mut saida = partes.join("\n\n");
    saida.push('\n');
    saida
}

/// Junta o comentário do bloco (se houver) ao texto da declaração.
fn bloco(chave: Chave, texto: String, comentarios: &BTreeMap<Chave, Vec<String>>) -> String {
    match comentarios.get(&chave) {
        Some(linhas) if !linhas.is_empty() => format!("{}\n{texto}", linhas.join("\n")),
        _ => texto,
    }
}

// ------------------------------------------------------------------ comentários

/// Lê os comentários ligados a declarações de topo, no texto de origem.
///
/// Texto que não parseia devolve mapa vazio: nesse caso `formatar` já parou antes.
fn comentarios(fonte: &str) -> BTreeMap<Chave, Vec<String>> {
    let mut mapa = BTreeMap::new();
    let Ok(modulo) = parse_ast(fonte, "fluxo.star") else {
        return mapa;
    };
    let Stmt::Statements(itens) = &modulo.statement().node else {
        return mapa;
    };
    for item in itens {
        let Some(chave) = chave_da_declaracao(item) else {
            continue;
        };
        let inicio = item.span.begin().get() as usize;
        let linhas = comentario_acima(fonte, inicio);
        if !linhas.is_empty() {
            mapa.insert(chave, linhas);
        }
    }
    mapa
}

/// De qual declaração de topo este trecho é, com o id que a IR usa.
fn chave_da_declaracao(item: &AstStmt) -> Option<Chave> {
    match &item.node {
        Stmt::Def(definicao) => Some(Chave::Funcao(definicao.name.node.ident.clone())),
        Stmt::Expression(expr) => {
            let Expr::Call(alvo, args) = &expr.node else {
                return None;
            };
            let Expr::Identifier(nome) = &alvo.node else {
                return None;
            };
            match nome.node.ident.as_str() {
                "fluxo" => Some(Chave::Fluxo),
                "etapa" => argumento_id(args).map(Chave::Etapa),
                "decisao" => argumento_id(args).map(Chave::Decisao),
                _ => None,
            }
        }
        _ => None,
    }
}

/// O texto do argumento nomeado `id = "..."`, quando é literal.
fn argumento_id(args: &CallArgsP<AstNoPayload>) -> Option<String> {
    for argumento in &args.args {
        if let Argument::Named(nome, expr) = &argumento.node
            && nome.node == "id"
            && let Expr::Literal(AstLiteral::String(texto)) = &expr.node
        {
            return Some(texto.node.clone());
        }
    }
    None
}

/// O bloco de linhas `#` imediatamente acima do byte `inicio`, já aparado e da primeira à última.
///
/// Para no primeiro que não é comentário (inclusive linha em branco): é o que separa "ligado" de
/// "solto".
fn comentario_acima(fonte: &str, inicio: usize) -> Vec<String> {
    let mut linhas: Vec<String> = Vec::new();
    let mut comeco_da_linha = match fonte.get(..inicio).and_then(|antes| antes.rfind('\n')) {
        Some(quebra) => quebra + 1,
        None => 0,
    };
    while comeco_da_linha > 0 {
        let fim_anterior = comeco_da_linha - 1;
        let comeco_anterior = match fonte
            .get(..fim_anterior)
            .and_then(|antes| antes.rfind('\n'))
        {
            Some(quebra) => quebra + 1,
            None => 0,
        };
        let Some(linha) = fonte.get(comeco_anterior..fim_anterior) else {
            break;
        };
        let limpa = linha.trim();
        if !limpa.starts_with('#') {
            break;
        }
        linhas.push(limpa.to_owned());
        comeco_da_linha = comeco_anterior;
    }
    linhas.reverse();
    linhas
}

// ------------------------------------------------------------------ documento

/// Um pedaço do script na forma canônica, antes de virar texto.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Valor {
    /// Texto já entre aspas e escapado (`"assim"`) ou booleano (`True`).
    Literal(String),
    /// Lista de valores.
    Lista(Lista),
    /// Chamada de declaração, de efeito ou de regra.
    Chamada(Chamada),
}

/// Lista de valores, com a decisão de abrir sempre (uma por linha) ou só quando não couber.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Lista {
    itens: Vec<Valor>,
    /// `true` para `campos`, `acoes` e `ao_entrar`: são a estrutura da etapa e ficam um por linha.
    aberta: bool,
}

/// Chamada com nome fixo e argumentos em ordem fixa.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Chamada {
    nome: &'static str,
    args: Vec<Argumento>,
}

/// Um argumento: posicional ou nomeado.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Argumento {
    nome: Option<&'static str>,
    valor: Valor,
}

fn chamada(nome: &'static str, args: Vec<Argumento>) -> Valor {
    Valor::Chamada(Chamada { nome, args })
}

fn posicional(valor: Valor) -> Argumento {
    Argumento { nome: None, valor }
}

fn nomeado(nome: &'static str, valor: Valor) -> Argumento {
    Argumento {
        nome: Some(nome),
        valor,
    }
}

/// Texto do script (com aspas) a partir do valor.
fn literal(texto: &str) -> Valor {
    Valor::Literal(texto_entre_aspas(texto))
}

/// O valor booleano do script.
fn booleano(valor: bool) -> Valor {
    Valor::Literal(if valor { "True" } else { "False" }.to_owned())
}

/// Lista que só abre quando não cabe na linha.
fn lista(itens: Vec<Valor>) -> Valor {
    Valor::Lista(Lista {
        itens,
        aberta: false,
    })
}

/// Lista que fica sempre aberta, um item por linha: `campos`, `acoes` e `ao_entrar` da etapa.
fn lista_aberta(itens: Vec<Valor>) -> Valor {
    Valor::Lista(Lista {
        itens,
        aberta: true,
    })
}

fn lista_de_textos(itens: &[String]) -> Valor {
    lista(itens.iter().map(|item| literal(item)).collect())
}

/// O texto entre aspas, com os escapes que o dialeto entende.
fn texto_entre_aspas(texto: &str) -> String {
    let mut saida = String::with_capacity(texto.len() + 2);
    saida.push('"');
    for caractere in texto.chars() {
        match caractere {
            '\\' => saida.push_str("\\\\"),
            '"' => saida.push_str("\\\""),
            '\n' => saida.push_str("\\n"),
            '\r' => saida.push_str("\\r"),
            '\t' => saida.push_str("\\t"),
            outro if outro.is_control() => {
                saida.push_str(&format!("\\u{:04x}", outro as u32));
            }
            outro => saida.push(outro),
        }
    }
    saida.push('"');
    saida
}

// ------------------------------------------------------------------ IR → documento

fn valor_do_fluxo(ir: &Ir) -> Valor {
    let mut args = vec![
        nomeado("id", literal(&ir.fluxo.id)),
        nomeado("nome", literal(&ir.fluxo.nome)),
    ];
    if let Some(descricao) = &ir.fluxo.descricao {
        args.push(nomeado("descricao", literal(descricao)));
    }
    args.push(nomeado("inicio", literal(&ir.start)));
    args.push(nomeado("gatilho", valor_do_gatilho(&ir.gatilho)));
    chamada("fluxo", args)
}

fn valor_do_gatilho(gatilho: &Gatilho) -> Valor {
    let mut args = vec![nomeado("quando", literal(&gatilho.when))];
    if let Some(nao_quando) = &gatilho.not_when {
        args.push(nomeado("nao_quando", literal(nao_quando)));
    }
    if !gatilho.examples.is_empty() {
        args.push(nomeado("exemplos", lista_de_textos(&gatilho.examples)));
    }
    chamada("gatilho", args)
}

fn valor_da_etapa(etapa: &Etapa) -> Valor {
    let mut args = vec![
        nomeado("id", literal(&etapa.id)),
        nomeado("nome", literal(&etapa.nome)),
    ];
    if let Some(instrucao) = &etapa.instruction {
        args.push(nomeado("instrucao", literal(instrucao)));
    }
    if etapa.action_actors != [ActionActor::Agent] {
        args.push(nomeado(
            "atores",
            lista(
                etapa
                    .action_actors
                    .iter()
                    .map(|ator| literal(ator.as_text()))
                    .collect(),
            ),
        ));
    }
    if !etapa.campos.is_empty() {
        args.push(nomeado(
            "campos",
            lista_aberta(etapa.campos.iter().map(valor_do_campo).collect()),
        ));
    }
    if !etapa.on_enter.is_empty() {
        args.push(nomeado(
            "ao_entrar",
            lista_aberta(etapa.on_enter.iter().map(valor_do_efeito).collect()),
        ));
    }
    if !etapa.actions.is_empty() {
        args.push(nomeado(
            "acoes",
            lista_aberta(etapa.actions.iter().map(valor_da_acao).collect()),
        ));
    }
    chamada("etapa", args)
}

fn valor_do_campo(campo: &Campo) -> Valor {
    let mut args = vec![
        posicional(literal(&campo.id)),
        posicional(literal(&campo.label)),
        nomeado("tipo", literal(campo.tipo.as_text())),
    ];
    if !campo.options.is_empty() {
        args.push(nomeado("opcoes", lista_de_textos(&campo.options)));
    }
    if campo.required {
        args.push(nomeado("obrigatorio", booleano(true)));
    }
    if let Some(ajuda) = &campo.help {
        args.push(nomeado("ajuda", literal(ajuda)));
    }
    chamada("campo", args)
}

fn valor_da_acao(acao: &Acao) -> Valor {
    let mut args = vec![
        posicional(literal(&acao.id)),
        posicional(literal(&acao.label)),
    ];
    if !acao.requires.is_empty() {
        args.push(nomeado("exige", lista_de_textos(&acao.requires)));
    }
    args.push(nomeado("vai_para", valor_do_destino(&acao.goes_to)));
    if let Some(ator) = acao.actor {
        args.push(nomeado("ator", literal(ator.as_text())));
    }
    if !acao.effects.is_empty() {
        args.push(nomeado(
            "efeitos",
            lista(acao.effects.iter().map(valor_do_efeito).collect()),
        ));
    }
    chamada("acao", args)
}

fn valor_do_destino(destino: &Destination) -> Valor {
    match destino {
        Destination::Etapa { id } | Destination::Decisao { id } => literal(id),
        Destination::Fim {
            id,
            resultado,
            ends_service: encerra_atendimento,
        } => valor_do_fim(id, resultado.as_deref(), *encerra_atendimento),
    }
}

fn valor_do_fim(id: &str, resultado: Option<&str>, encerra_atendimento: bool) -> Valor {
    let mut args = vec![posicional(literal(id))];
    if let Some(resultado) = resultado {
        args.push(nomeado("resultado", literal(resultado)));
    }
    if encerra_atendimento {
        args.push(nomeado(
            "encerra_atendimento",
            booleano(encerra_atendimento),
        ));
    }
    chamada("fim", args)
}

fn valor_da_decisao(decisao: &Decisao) -> Valor {
    chamada(
        "decisao",
        vec![
            nomeado("id", literal(&decisao.id)),
            nomeado("se", valor_da_regra(&decisao.se)),
            nomeado("entao", valor_do_destino(&decisao.then_branch)),
            nomeado("senao", valor_do_destino(&decisao.else_branch)),
        ],
    )
}

fn valor_da_regra(regra: &Rule) -> Valor {
    match regra {
        Rule::Igual {
            campo,
            value: valor,
        } => dois_textos("igual", campo, valor),
        Rule::Diferente {
            campo,
            value: valor,
        } => dois_textos("diferente", campo, valor),
        Rule::Maior {
            campo,
            value: valor,
        } => dois_textos("maior", campo, valor),
        Rule::Menor {
            campo,
            value: valor,
        } => dois_textos("menor", campo, valor),
        Rule::Contem {
            campo,
            value: valor,
        } => dois_textos("contem", campo, valor),
        Rule::Vazio { campo } => chamada("vazio", vec![posicional(literal(campo))]),
        Rule::E { args } => combinador("e", args),
        Rule::Ou { args } => combinador("ou", args),
        Rule::Nao { args } => combinador("nao", args),
        Rule::Funcao { nome } => chamada("funcao", vec![posicional(literal(nome))]),
    }
}

fn dois_textos(nome: &'static str, campo: &str, valor: &str) -> Valor {
    chamada(
        nome,
        vec![posicional(literal(campo)), posicional(literal(valor))],
    )
}

fn combinador(nome: &'static str, regras: &[Rule]) -> Valor {
    chamada(
        nome,
        regras
            .iter()
            .map(|regra| posicional(valor_da_regra(regra)))
            .collect(),
    )
}

fn valor_do_efeito(efeito: &Effect) -> Valor {
    match efeito {
        Effect::MessageToCustomer { texto } => {
            chamada("mensagem_ao_cliente", vec![posicional(literal(texto))])
        }
        Effect::NotifyWhatsApp { para, mensagem } => chamada(
            "notificar_whatsapp",
            vec![
                nomeado("para", literal(para)),
                nomeado("mensagem", literal(mensagem)),
            ],
        ),
        Effect::Encaminhar { departamento } => {
            chamada("encaminhar", vec![posicional(literal(departamento))])
        }
    }
}

// ------------------------------------------------------------------ impressão

/// Escreve uma declaração de topo: sempre aberta, um argumento por linha.
fn corpo(valor: &Valor) -> String {
    let mut saida = String::new();
    escreve_aberta(valor, 0, &mut saida);
    saida
}

fn escreve_aberta(valor: &Valor, recuo: usize, saida: &mut String) {
    let Valor::Chamada(chamada) = valor else {
        escreve(valor, recuo, recuo, saida);
        return;
    };
    if chamada.args.is_empty() {
        saida.push_str(chamada.nome);
        saida.push_str("()");
        return;
    }
    saida.push_str(chamada.nome);
    saida.push_str("(\n");
    let recuo_interno = recuo + RECUO;
    for argumento in &chamada.args {
        saida.push_str(&" ".repeat(recuo_interno));
        escreve_argumento(argumento, recuo_interno, saida);
        saida.push_str(",\n");
    }
    saida.push_str(&" ".repeat(recuo));
    saida.push(')');
}

fn escreve_argumento(argumento: &Argumento, recuo: usize, saida: &mut String) {
    let linha = argumento_em_uma_linha(argumento);
    if cabe(&linha, recuo) && !aberta_sempre(&argumento.valor) {
        saida.push_str(&linha);
        return;
    }
    let prefixo = match argumento.nome {
        Some(nome) => {
            saida.push_str(nome);
            saida.push_str(" = ");
            nome.chars().count() + 3
        }
        None => 0,
    };
    escreve(&argumento.valor, recuo, recuo + prefixo, saida);
}

/// Escreve um valor, em uma linha quando cabe (e quando a regra não obriga a abrir).
fn escreve(valor: &Valor, recuo: usize, coluna: usize, saida: &mut String) {
    let linha = valor_em_uma_linha(valor);
    if cabe(&linha, coluna) && !aberta_sempre(valor) {
        saida.push_str(&linha);
        return;
    }
    match valor {
        // Texto não se parte: quebrar um literal mudaria o valor.
        Valor::Literal(_) => saida.push_str(&linha),
        Valor::Lista(lista) => {
            if lista.itens.is_empty() {
                saida.push_str("[]");
                return;
            }
            saida.push_str("[\n");
            let recuo_interno = recuo + RECUO;
            for item in &lista.itens {
                saida.push_str(&" ".repeat(recuo_interno));
                escreve(item, recuo_interno, recuo_interno, saida);
                saida.push_str(",\n");
            }
            saida.push_str(&" ".repeat(recuo));
            saida.push(']');
        }
        Valor::Chamada(_) => escreve_aberta(valor, recuo, saida),
    }
}

/// O valor em uma linha só.
fn valor_em_uma_linha(valor: &Valor) -> String {
    match valor {
        Valor::Literal(texto) => texto.clone(),
        Valor::Lista(lista) => {
            let dentro: Vec<String> = lista.itens.iter().map(valor_em_uma_linha).collect();
            format!("[{}]", dentro.join(", "))
        }
        Valor::Chamada(chamada) => {
            let dentro: Vec<String> = chamada.args.iter().map(argumento_em_uma_linha).collect();
            format!("{}({})", chamada.nome, dentro.join(", "))
        }
    }
}

fn argumento_em_uma_linha(argumento: &Argumento) -> String {
    match argumento.nome {
        Some(nome) => format!("{nome} = {}", valor_em_uma_linha(&argumento.valor)),
        None => valor_em_uma_linha(&argumento.valor),
    }
}

/// O valor que não fica em uma linha, mesmo quando caberia.
///
/// Regra composta (`e`, `ou`, `nao`) se lê como lista, e `campos`/`acoes` de uma etapa são a
/// estrutura dela: um item por linha.
fn aberta_sempre(valor: &Valor) -> bool {
    match valor {
        Valor::Lista(lista) => lista.aberta,
        Valor::Chamada(chamada) => matches!(chamada.nome, "e" | "ou" | "nao"),
        Valor::Literal(_) => false,
    }
}

/// Se o trecho, começando na coluna dada, cabe na largura canônica.
fn cabe(texto: &str, coluna: usize) -> bool {
    coluna + texto.chars().count() <= LARGURA
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::ir::Fim;
    use poph_types::FieldKind;

    #[test]
    fn o_texto_entre_aspas_escapa_o_que_o_dialeto_interpreta() {
        assert_eq!(texto_entre_aspas("simples"), "\"simples\"");
        assert_eq!(texto_entre_aspas("com \"aspas\""), "\"com \\\"aspas\\\"\"");
        assert_eq!(
            texto_entre_aspas("barra \\ invertida"),
            "\"barra \\\\ invertida\""
        );
        assert_eq!(
            texto_entre_aspas("linha\nnova\ttab"),
            "\"linha\\nnova\\ttab\""
        );
        assert_eq!(texto_entre_aspas("acento ção"), "\"acento ção\"");
        assert_eq!(texto_entre_aspas("\u{1}"), "\"\\u0001\"");
    }

    #[test]
    fn a_largura_conta_caracteres_e_nao_bytes() {
        assert!(cabe(&"ç".repeat(LARGURA), 0));
        assert!(!cabe(&"ç".repeat(LARGURA + 1), 0));
        assert!(cabe("abc", LARGURA - 3));
        assert!(!cabe("abcd", LARGURA - 3));
    }

    #[test]
    fn o_comentario_acima_pega_so_o_bloco_encostado() {
        let fonte = "# primeiro\n# segundo\nfluxo(\n    id = \"x\",\n)\n";
        assert_eq!(
            comentario_acima(fonte, fonte.find("fluxo").unwrap()),
            vec!["# primeiro".to_owned(), "# segundo".to_owned()]
        );
        // Linha em branco no meio quebra o bloco: só a linha encostada conta.
        let com_buraco = "# solto\n\n# ligado\nfluxo(\n)\n";
        assert_eq!(
            comentario_acima(com_buraco, com_buraco.find("fluxo").unwrap()),
            vec!["# ligado".to_owned()]
        );
        // Declaração na primeira linha não tem comentário acima.
        assert!(comentario_acima("fluxo()\n", 0).is_empty());
    }

    #[test]
    fn a_chave_da_declaracao_le_o_id_do_argumento() {
        let fonte = "fluxo(id = \"x\")\netapa(id = \"passo\", nome = \"P\")\ndecisao(id = \"d\", se = vazio(\"a\"), entao = \"passo\", senao = \"passo\")\ndef regra(dados):\n    return True\n";
        let modulo = parse_ast(fonte, "fluxo.star").expect("parseia");
        let Stmt::Statements(itens) = &modulo.statement().node else {
            panic!("o topo é uma lista de declarações");
        };
        let chaves: Vec<Chave> = itens.iter().filter_map(chave_da_declaracao).collect();
        assert_eq!(
            chaves,
            vec![
                Chave::Fluxo,
                Chave::Etapa("passo".to_owned()),
                Chave::Decisao("d".to_owned()),
                Chave::Funcao("regra".to_owned()),
            ]
        );
    }

    #[test]
    fn os_booleanos_e_os_numeros_do_campo_saem_do_jeito_do_script() {
        let campo = Campo {
            id: "idade".to_owned(),
            label: "Idade".to_owned(),
            tipo: FieldKind::Number,
            required: false,
            options: Vec::new(),
            help: None,
        };
        assert_eq!(
            valor_em_uma_linha(&valor_do_campo(&campo)),
            "campo(\"idade\", \"Idade\", tipo = \"numero\")"
        );
    }

    #[test]
    fn a_defe_impressa_e_a_fonte_que_a_ir_guardou() {
        let ir = Ir {
            schema_versao: crate::ir::SCHEMA_VERSAO,
            fluxo: crate::ir::Fluxo {
                id: "com_def".to_owned(),
                nome: "Com def".to_owned(),
                descricao: None,
            },
            gatilho: Gatilho {
                when: "quando".to_owned(),
                not_when: None,
                examples: Vec::new(),
            },
            start: "unica".to_owned(),
            stages: vec![Etapa {
                id: "unica".to_owned(),
                nome: "Única".to_owned(),
                instruction: None,
                action_actors: vec![ActionActor::Agent],
                campos: Vec::new(),
                actions: vec![Acao {
                    id: "seguir".to_owned(),
                    label: "Seguir".to_owned(),
                    requires: Vec::new(),
                    actor: None,
                    goes_to: Destination::Fim {
                        id: "fim".to_owned(),
                        resultado: None,
                        ends_service: false,
                    },
                    effects: Vec::new(),
                }],
                on_enter: Vec::new(),
            }],
            decisions: Vec::new(),
            ends: vec![Fim {
                id: "fim".to_owned(),
                resultado: None,
                ends_service: false,
            }],
            defs: vec![crate::ir::Def {
                nome: "regra".to_owned(),
                fonte: "def regra(dados):\n    return True".to_owned(),
            }],
        };
        let texto = imprimir(&ir);
        assert!(
            texto.contains("def regra(dados):\n    return True"),
            "a def tem de sair literal:\n{texto}"
        );
        // A `def` vai antes de quem a usa (aqui, antes da etapa).
        let posicao_def = texto.find("def regra").expect("def impressa");
        let posicao_etapa = texto.find("etapa(").expect("etapa impressa");
        assert!(posicao_def < posicao_etapa, "def depois da etapa:\n{texto}");
    }

    #[test]
    fn a_lista_de_fins_da_ir_se_imprime_igual() {
        let fim = Fim {
            id: "desistiu".to_owned(),
            resultado: Some("Cliente desistiu".to_owned()),
            ends_service: true,
        };
        assert_eq!(
            valor_em_uma_linha(&valor_do_fim(
                &fim.id,
                fim.resultado.as_deref(),
                fim.ends_service
            )),
            "fim(\"desistiu\", resultado = \"Cliente desistiu\", encerra_atendimento = True)"
        );
    }
}
