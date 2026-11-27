//! Validador semântico do fluxo (FX-11): referências, ids, opções, atores, regras, variáveis,
//! alcançabilidade, ciclos e tetos, com a lista fechada de diagnósticos (código, severidade,
//! mensagem em português e sugestão).
//!
//! São **duas** portas, e elas existem por motivos diferentes:
//!
//! - [`validar`] confere uma **IR já compilada** (estrutura, não texto). Serve para a estrutura que
//!   não passou pelo compilador: importação de fluxo, revisão antiga, IR que alguém gravou à mão,
//!   `resolucoes_json` conferido na publicação. Como não há texto, cada diagnóstico aponta o
//!   **caminho do nó** (`etapas[2] (id "entrega") > acoes[1] > vai_para`) em vez de linha e coluna.
//! - [`resolver_departamentos`] traduz o **nome** de departamento citado em `encaminhar(...)` para o
//!   id do departamento na organização, e avisa (`FLX65`) quando o nome não casa com nenhum ou casa
//!   com mais de um. Quem grava o resultado em `fluxo_revisoes.resolucoes_json` é a publicação
//!   (FX-09); aqui a lista de departamentos da org entra por parâmetro.
//!
//! A fronteira com o compilador (FX-10): os códigos que dependem de **posição no texto** nascem no
//! compilador (que lê a árvore sintática) e são reemitidos aqui **sem** posição quando a IR vem de
//! fora; os de **grafo** (ciclo, alcance), os de **uso** (campo, `def`, ator) e o teto de nós são
//! conferidos aqui também, porque sem isso uma IR de fora passaria com fluxo quebrado.
//!
//! Tudo puro: sem banco, sem rede, sem relógio, sem arquivo. Duas chamadas com a mesma IR devolvem
//! a mesma lista, na mesma ordem.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use poph_types::FieldKind;

use crate::diagnostics::{Code, Diagnostic};
use crate::ir::{Destination, Effect, Etapa, Ir, Rule};

/// Teto de nós (etapas + decisões + fins), `FLUXOS_LINGUAGEM.md` §3.4 — o mesmo do compilador.
pub const MAX_NOS: usize = crate::compilador::form::MAX_NOS;

/// Um departamento da organização, como o validador precisa dele: id e nome.
///
/// Vem do `poph-store` por parâmetro (o crate é puro); quem chama é a publicação.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Departamento {
    /// Id do departamento na organização (`dep_...`).
    pub id: String,
    /// Nome que o autor do script escreve em `encaminhar("...")`.
    pub nome: String,
}

/// O id que corresponde ao nome citado em `encaminhar(...)`.
///
/// `departamento_id` é `None` quando o nome não casou com nenhum departamento ou casou com mais de
/// um: nesse caso há um aviso `FLX65` e o encaminhamento vai falhar no atendimento (o efeito fica
/// `falhou`, com evento na Ficha).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolucao {
    /// Nome como o script escreveu (sem as pontas).
    pub nome: String,
    /// Id do departamento na organização; `None` quando não houve casamento único.
    pub departamento_id: Option<String>,
}

/// Confere uma IR já compilada e devolve os diagnósticos, na ordem dos nós da IR.
///
/// Devolve também os **avisos** (publicar não exige zero aviso, mas o autor deve ver). A IR que veio
/// do compilador já passou por estas conferências: chamar isto sobre ela serve para provar que as
/// duas direções concordam (há teste para isso).
pub fn validar(ir: &Ir) -> Vec<Diagnostic> {
    let mut validador = Validador::novo(ir);
    validador.confere_etapas();
    validador.confere_decisoes();
    validador.confere_fins();
    validador.confere_inicio();
    validador.confere_ciclo();
    validador.confere_alcance();
    validador.confere_teto();
    validador.confere_uso_dos_campos();
    validador.confere_uso_das_defs();
    validador.diagnosticos
}

/// Traduz os nomes de departamento citados em `encaminhar(...)` para os ids da organização.
///
/// Devolve uma [`Resolucao`] por nome citado (na ordem da primeira citação na IR) e um aviso
/// `FLX65` para cada nome que não casa com um departamento **único**. Nome que não casa continua na
/// lista, com `departamento_id = None`: a publicação grava o que resolveu e o atendimento sabe o que
/// vai falhar.
///
/// O casamento ignora espaços nas pontas e maiúscula/minúscula; acento **conta** (`Logística` não é
/// `Logistica`).
pub fn resolver_departamentos(
    ir: &Ir,
    departamentos: &[Departamento],
) -> (Vec<Resolucao>, Vec<Diagnostic>) {
    let mut citacoes: Vec<(String, String)> = Vec::new();
    let mut vistos: BTreeSet<String> = BTreeSet::new();
    let mut citar = |nome: &str, caminho: String| {
        let chave = mesma_chave(nome);
        if vistos.insert(chave) {
            citacoes.push((nome.trim().to_owned(), caminho));
        }
    };

    for (i, etapa) in ir.stages.iter().enumerate() {
        let base = caminho_etapa(i, etapa);
        for (j, efeito) in etapa.on_enter.iter().enumerate() {
            if let Effect::Encaminhar { departamento } = efeito {
                citar(departamento, sub(&base, &format!("ao_entrar[{j}]")));
            }
        }
        for (j, acao) in etapa.actions.iter().enumerate() {
            let caminho_acao = sub(&base, &format!("acoes[{j}]"));
            for (k, efeito) in acao.effects.iter().enumerate() {
                if let Effect::Encaminhar { departamento } = efeito {
                    citar(departamento, sub(&caminho_acao, &format!("efeitos[{k}]")));
                }
            }
        }
    }

    let mut resolucoes = Vec::with_capacity(citacoes.len());
    let mut diagnosticos = Vec::new();
    for (nome, caminho) in citacoes {
        let achados: Vec<&Departamento> = departamentos
            .iter()
            .filter(|departamento| mesma_chave(&departamento.nome) == mesma_chave(&nome))
            .collect();
        let resolucao = match achados.as_slice() {
            [unico] => Resolucao {
                nome,
                departamento_id: Some(unico.id.clone()),
            },
            [] => {
                diagnosticos.push(Diagnostic::warning_at(
                    Code::UnknownDepartment,
                    format!(
                        "O encaminhamento manda o atendimento para o departamento `{nome}`, que não existe nesta organização: o efeito vai falhar no atendimento. Confira o nome do departamento."
                    ),
                    caminho,
                ));
                Resolucao {
                    nome,
                    departamento_id: None,
                }
            }
            varios => {
                diagnosticos.push(Diagnostic::warning_at(
                    Code::UnknownDepartment,
                    format!(
                        "O encaminhamento manda o atendimento para `{nome}`, mas há {} departamentos com esse nome na organização: renomeie um deles para o encaminhamento ter destino. O efeito vai falhar no atendimento.",
                        varios.len()
                    ),
                    caminho,
                ));
                Resolucao {
                    nome,
                    departamento_id: None,
                }
            }
        };
        resolucoes.push(resolucao);
    }

    (resolucoes, diagnosticos)
}

// ------------------------------------------------------------------ caminhos dos nós

/// Caminho de uma etapa na IR: `etapas[2] (id "entrega")`.
fn caminho_etapa(indice: usize, etapa: &Etapa) -> String {
    format!("etapas[{indice}] (id \"{}\")", etapa.id)
}

/// Caminho de uma decisão na IR: `decisoes[0] (id "classificar")`.
fn caminho_decisao(indice: usize, decisao: &crate::ir::Decisao) -> String {
    format!("decisoes[{indice}] (id \"{}\")", decisao.id)
}

/// Desce um pedaço no caminho: `etapas[0] (id "x") > acoes[1]`.
fn sub(base: &str, pedaco: &str) -> String {
    if base.is_empty() {
        pedaco.to_owned()
    } else {
        format!("{base} > {pedaco}")
    }
}

/// A chave de comparação de nome de departamento: sem espaços nas pontas, sem diferença de caixa.
fn mesma_chave(nome: &str) -> String {
    nome.trim().to_lowercase()
}

// ------------------------------------------------------------------ o validador

struct Validador<'a> {
    ir: &'a Ir,
    /// Primeira posição de cada id de etapa (id repetido é apontado nas seguintes).
    etapas: BTreeMap<&'a str, usize>,
    decisoes: BTreeMap<&'a str, usize>,
    fins: BTreeMap<&'a str, usize>,
    /// Primeira posição de cada `def`, por nome.
    defs: BTreeMap<&'a str, usize>,
    /// Campos lidos por regra, por etapa.
    lidos: BTreeMap<&'a str, BTreeSet<&'a str>>,
    /// Campos exigidos por ação, por etapa.
    exigidos: BTreeMap<&'a str, BTreeSet<&'a str>>,
    /// `def` usadas por alguma decisão.
    usadas: BTreeSet<&'a str>,
    diagnosticos: Vec<Diagnostic>,
}

impl<'a> Validador<'a> {
    fn novo(ir: &'a Ir) -> Self {
        let mut validador = Self {
            ir,
            etapas: BTreeMap::new(),
            decisoes: BTreeMap::new(),
            fins: BTreeMap::new(),
            defs: BTreeMap::new(),
            lidos: BTreeMap::new(),
            exigidos: BTreeMap::new(),
            usadas: BTreeSet::new(),
            diagnosticos: Vec::new(),
        };
        for (i, etapa) in ir.stages.iter().enumerate() {
            validador.etapas.entry(etapa.id.as_str()).or_insert(i);
        }
        for (i, decisao) in ir.decisions.iter().enumerate() {
            validador.decisoes.entry(decisao.id.as_str()).or_insert(i);
        }
        for (i, fim) in ir.ends.iter().enumerate() {
            validador.fins.entry(fim.id.as_str()).or_insert(i);
        }
        for (i, def) in ir.defs.iter().enumerate() {
            validador.defs.entry(def.nome.as_str()).or_insert(i);
        }
        validador
    }

    fn erro_no(&mut self, codigo: Code, mensagem: impl Into<String>, caminho: impl Into<String>) {
        self.diagnosticos
            .push(Diagnostic::error_at(codigo, mensagem, caminho));
    }

    fn aviso_no(&mut self, codigo: Code, mensagem: impl Into<String>, caminho: impl Into<String>) {
        self.diagnosticos
            .push(Diagnostic::warning_at(codigo, mensagem, caminho));
    }

    // -------------------------------------------------------------- etapas

    fn confere_etapas(&mut self) {
        let ir = self.ir;
        for (i, etapa) in ir.stages.iter().enumerate() {
            let base = caminho_etapa(i, etapa);
            if self.etapas.get(etapa.id.as_str()) != Some(&i) {
                self.erro_no(
                    Code::DuplicateId,
                    format!(
                        "O id `{}` está declarado duas vezes. Cada etapa, decisão, campo e ação tem id único no seu escopo.",
                        etapa.id
                    ),
                    base.clone(),
                );
            } else if self.decisoes.contains_key(etapa.id.as_str()) {
                self.erro_no(
                    Code::DuplicateId,
                    format!(
                        "O id `{}` é de uma etapa e de uma decisão ao mesmo tempo: um destino não saberia para onde ir. Use ids diferentes.",
                        etapa.id
                    ),
                    base.clone(),
                );
            }

            self.confere_campos(etapa, &base);
            self.confere_acoes(etapa, &base);
            self.confere_variaveis(etapa, &base);

            if etapa.actions.is_empty() {
                self.aviso_no(
                    Code::StageWithoutAction,
                    format!(
                        "A etapa `{}` não tem nenhuma `acao(...)`: o atendimento não sai dela. Acrescente uma ação com `vai_para`.",
                        etapa.id
                    ),
                    base.clone(),
                );
            }

            // Ator permitido na etapa sem nenhuma ação dele: só vale quando toda ação é restrita
            // (ação sem `ator` é de qualquer ator da etapa).
            if !etapa.actions.is_empty() && etapa.actions.iter().all(|acao| acao.actor.is_some()) {
                for ator in &etapa.action_actors {
                    if etapa.actions.iter().any(|acao| acao.actor == Some(*ator)) {
                        continue;
                    }
                    self.aviso_no(
                        Code::ActorWithoutAction,
                        format!(
                            "A etapa `{}` permite o `{}`, mas nenhuma ação é dele: na prática ele não tem o que fazer aqui.",
                            etapa.id,
                            ator.as_text()
                        ),
                        sub(&base, "atores_acao"),
                    );
                }
            }
        }
    }

    fn confere_campos(&mut self, etapa: &'a Etapa, base: &str) {
        let mut ids: BTreeSet<&str> = BTreeSet::new();
        for (j, campo) in etapa.campos.iter().enumerate() {
            let caminho = sub(base, &format!("campos[{j}]"));
            if !ids.insert(campo.id.as_str()) {
                self.erro_no(
                    Code::DuplicateId,
                    format!(
                        "O campo `{}` está declarado duas vezes na etapa `{}`.",
                        campo.id, etapa.id
                    ),
                    caminho.clone(),
                );
            }
            match campo.tipo {
                FieldKind::Option => {
                    if campo.options.len() < 2 {
                        self.erro_no(
                            Code::InvalidOptions,
                            format!(
                                "O campo `{}.{}` é do tipo `opcao` e precisa de pelo menos duas opções; hoje tem {}.",
                                etapa.id,
                                campo.id,
                                campo.options.len()
                            ),
                            caminho,
                        );
                    } else if repetidas(&campo.options) {
                        self.erro_no(
                            Code::InvalidOptions,
                            format!(
                                "O campo `{}.{}` repete uma opção: as opções precisam ser diferentes entre si.",
                                etapa.id, campo.id
                            ),
                            caminho,
                        );
                    }
                }
                _ if !campo.options.is_empty() => {
                    self.erro_no(
                        Code::InvalidOptions,
                        format!(
                            "O campo `{}.{}` é do tipo `{}` e não aceita `opcoes`: isso só vale no tipo `opcao`.",
                            etapa.id,
                            campo.id,
                            campo.tipo.as_text()
                        ),
                        caminho,
                    );
                }
                _ => {}
            }
        }
    }

    fn confere_acoes(&mut self, etapa: &'a Etapa, base: &str) {
        let mut ids: BTreeSet<&str> = BTreeSet::new();
        for (j, acao) in etapa.actions.iter().enumerate() {
            let caminho = sub(base, &format!("acoes[{j}]"));
            if !ids.insert(acao.id.as_str()) {
                self.erro_no(
                    Code::DuplicateId,
                    format!(
                        "A ação `{}` está declarada duas vezes na etapa `{}`.",
                        acao.id, etapa.id
                    ),
                    caminho.clone(),
                );
            }
            let onde = format!("A ação `{}.{}`", etapa.id, acao.id);

            for exigido in &acao.requires {
                if etapa.campos.iter().any(|campo| campo.id == *exigido) {
                    self.exigidos
                        .entry(etapa.id.as_str())
                        .or_default()
                        .insert(exigido.as_str());
                } else {
                    self.erro_no(
                        Code::UnknownRequirement,
                        format!(
                            "{onde} exige `{exigido}`, que não é campo da etapa `{}`.",
                            etapa.id
                        ),
                        sub(&caminho, "exige"),
                    );
                }
            }

            if let Some(ator) = acao.actor
                && !etapa.action_actors.contains(&ator)
            {
                let permitidos: Vec<&str> =
                    etapa.action_actors.iter().map(|a| a.as_text()).collect();
                self.erro_no(
                    Code::InvalidRuleOrActor,
                    format!(
                        "{onde} é do ator `{}`, que não está em `atores` da etapa `{}` (lá estão: {}).",
                        ator.as_text(),
                        etapa.id,
                        if permitidos.is_empty() {
                            "nenhum".to_owned()
                        } else {
                            permitidos.join(", ")
                        }
                    ),
                    sub(&caminho, "ator"),
                );
            }

            self.confere_destino(&acao.goes_to, &sub(&caminho, "vai_para"), &onde);
        }
    }

    /// Confere as variáveis `{...}` das mensagens: formato (`FLX25`), existência do campo
    /// (`FLX59`) e variável do próprio passo em `ao_entrar` (`FLX63`).
    ///
    /// A varredura do texto é a mesma do compilador ([`crate::compilador::form::variables`]), para
    /// as duas direções não divergirem.
    fn confere_variaveis(&mut self, etapa: &'a Etapa, base: &str) {
        for (j, efeito) in etapa.on_enter.iter().enumerate() {
            let caminho = sub(base, &format!("ao_entrar[{j}]"));
            self.confere_variaveis_do_efeito(efeito, &caminho, Some(etapa));
        }
        for (j, acao) in etapa.actions.iter().enumerate() {
            let caminho_acao = sub(base, &format!("acoes[{j}]"));
            for (k, efeito) in acao.effects.iter().enumerate() {
                let caminho = sub(&caminho_acao, &format!("efeitos[{k}]"));
                self.confere_variaveis_do_efeito(efeito, &caminho, None);
            }
        }
    }

    fn confere_variaveis_do_efeito(
        &mut self,
        efeito: &Effect,
        caminho: &str,
        ao_entrar_da_etapa: Option<&'a Etapa>,
    ) {
        let (texto, campo, o_que) = match efeito {
            Effect::MessageToCustomer { texto } => (texto.as_str(), "texto", "mensagem ao cliente"),
            Effect::NotifyWhatsApp { mensagem, .. } => {
                (mensagem.as_str(), "mensagem", "notificação para a equipe")
            }
            Effect::Encaminhar { .. } => return,
        };
        let caminho = sub(caminho, campo);
        let ir = self.ir;
        for variavel in crate::compilador::form::variables(texto) {
            let Some((prefixo, nome)) = variavel.split_once('.') else {
                self.erro_no(
                    Code::InvalidVariable,
                    format!(
                        "A {o_que} usa `{{{variavel}}}`, que não existe. As variáveis são `{{etapa.campo}}`, `{{ficha.id}}` e `{{cliente.peer}}`."
                    ),
                    caminho.clone(),
                );
                continue;
            };
            if variavel == "ficha.id" || variavel == "cliente.peer" {
                continue;
            }
            if prefixo == "ficha" || prefixo == "cliente" {
                self.erro_no(
                    Code::InvalidVariable,
                    format!(
                        "A {o_que} usa `{{{variavel}}}`, que não existe. As variáveis são `{{etapa.campo}}`, `{{ficha.id}}` e `{{cliente.peer}}`."
                    ),
                    caminho.clone(),
                );
                continue;
            }
            let Some(indice) = self.etapas.get(prefixo).copied() else {
                self.aviso_no(
                    Code::VariableWithoutField,
                    format!(
                        "A {o_que} usa `{{{variavel}}}`, mas a etapa `{prefixo}` não existe: no atendimento sai vazio."
                    ),
                    caminho.clone(),
                );
                continue;
            };
            if !ir.stages[indice].campos.iter().any(|c| c.id == nome) {
                self.aviso_no(
                    Code::VariableWithoutField,
                    format!(
                        "A {o_que} usa `{{{variavel}}}`, mas a etapa `{prefixo}` não tem o campo `{nome}`: no atendimento sai vazio."
                    ),
                    caminho.clone(),
                );
                continue;
            }
            if let Some(atual) = ao_entrar_da_etapa
                && atual.id == prefixo
            {
                self.aviso_no(
                    Code::VariableFromOwnStep,
                    format!(
                        "`ao_entrar` da etapa `{}` usa `{{{variavel}}}`, que só é preenchido dentro da própria etapa: a mensagem sai com o texto vazio.",
                        atual.id
                    ),
                    caminho.clone(),
                );
            }
        }
    }

    // -------------------------------------------------------------- decisões e regras

    fn confere_decisoes(&mut self) {
        let ir = self.ir;
        for (i, decisao) in ir.decisions.iter().enumerate() {
            let base = caminho_decisao(i, decisao);
            if self.decisoes.get(decisao.id.as_str()) != Some(&i) {
                self.erro_no(
                    Code::DuplicateId,
                    format!(
                        "O id `{}` está declarado duas vezes entre as decisões do fluxo.",
                        decisao.id
                    ),
                    base.clone(),
                );
            } else if self.etapas.contains_key(decisao.id.as_str()) {
                self.erro_no(
                    Code::DuplicateId,
                    format!(
                        "O id `{}` é de uma decisão e de uma etapa ao mesmo tempo: um destino não saberia para onde ir. Use ids diferentes.",
                        decisao.id
                    ),
                    base.clone(),
                );
            }
            let onde = format!("A decisão `{}`", decisao.id);
            self.confere_destino(&decisao.then_branch, &sub(&base, "entao"), &onde);
            self.confere_destino(&decisao.else_branch, &sub(&base, "senao"), &onde);
            self.confere_regra(&decisao.se, &sub(&base, "se"), &onde);
        }
    }

    fn confere_regra(&mut self, regra: &'a Rule, caminho: &str, onde: &str) {
        match regra {
            Rule::Igual {
                campo,
                value: valor,
            } => self.confere_folha(campo, Some(valor), "igual", caminho, onde),
            Rule::Diferente {
                campo,
                value: valor,
            } => self.confere_folha(campo, Some(valor), "diferente", caminho, onde),
            Rule::Maior {
                campo,
                value: valor,
            } => self.confere_folha(campo, Some(valor), "maior", caminho, onde),
            Rule::Menor {
                campo,
                value: valor,
            } => self.confere_folha(campo, Some(valor), "menor", caminho, onde),
            Rule::Contem {
                campo,
                value: valor,
            } => self.confere_folha(campo, Some(valor), "contem", caminho, onde),
            Rule::Vazio { campo } => self.confere_folha(campo, None, "vazio", caminho, onde),
            Rule::Funcao { nome } => {
                self.usadas.insert(nome.as_str());
                if !self.defs.contains_key(nome.as_str()) {
                    self.erro_no(
                        Code::InvalidRuleOrActor,
                        format!("{onde} usa `funcao(\"{nome}\")`, mas não existe `def {nome}(dados)` neste fluxo."),
                        caminho,
                    );
                }
            }
            Rule::E { args } | Rule::Ou { args } | Rule::Nao { args } => {
                self.confere_grupo(regra, args, caminho, onde);
            }
        }
    }

    fn confere_grupo(&mut self, regra: &'a Rule, args: &'a [Rule], caminho: &str, onde: &str) {
        if matches!(regra, Rule::Nao { .. }) && args.len() != 1 {
            self.erro_no(
                Code::MalformedCombinator,
                format!(
                    "`nao` recebe exatamente uma condição; {onde} usa {}.",
                    args.len()
                ),
                caminho,
            );
        }
        if matches!(regra, Rule::E { .. } | Rule::Ou { .. }) && args.len() < 2 {
            self.erro_no(
                Code::MalformedCombinator,
                format!(
                    "`{}` precisa de pelo menos duas condições; {onde} usa {}.",
                    if matches!(regra, Rule::E { .. }) {
                        "e"
                    } else {
                        "ou"
                    },
                    args.len()
                ),
                caminho,
            );
        }

        // FLX55: condição que sempre dá o mesmo resultado.
        let folhas: Vec<(&str, Option<&str>, &str)> =
            args.iter().filter_map(chave_da_folha).collect();
        if matches!(regra, Rule::E { .. }) {
            for (i, a) in folhas.iter().enumerate() {
                for b in folhas.iter().skip(i + 1) {
                    let opostas =
                        matches!((a.2, b.2), ("igual", "diferente") | ("diferente", "igual"));
                    if a.0 == b.0 && a.1 == b.1 && opostas {
                        self.aviso_no(
                            Code::ContradictoryCondition,
                            format!(
                                "{onde} não pode ser verdadeira com `{}` igual e diferente de `{}` ao mesmo tempo.",
                                a.0,
                                a.1.unwrap_or("")
                            ),
                            caminho,
                        );
                        break;
                    }
                }
            }
        }
        if matches!(regra, Rule::Ou { .. }) {
            for (i, a) in args.iter().enumerate() {
                if args.iter().skip(i + 1).any(|b| b == a) {
                    self.aviso_no(
                        Code::ContradictoryCondition,
                        format!(
                            "{onde} repete o mesmo teste duas vezes na mesma disjunção (`ou`)."
                        ),
                        caminho,
                    );
                    break;
                }
            }
        }

        for (k, filho) in args.iter().enumerate() {
            self.confere_regra(filho, &sub(caminho, &format!("args[{k}]")), onde);
        }
    }

    fn confere_folha(
        &mut self,
        campo: &str,
        valor: Option<&str>,
        tipo: &str,
        caminho: &str,
        onde: &str,
    ) {
        let Some((etapa_id, campo_id)) = campo.split_once('.') else {
            self.erro_no(
                Code::MalformedCombinator,
                format!(
                    "{onde} compara `{campo}`; use `etapa.campo`, por exemplo `coleta.endereco`."
                ),
                caminho,
            );
            return;
        };
        if etapa_id.is_empty() || campo_id.is_empty() || campo_id.contains('.') {
            self.erro_no(
                Code::MalformedCombinator,
                format!(
                    "{onde} compara `{campo}`; use `etapa.campo`, com ids em minúsculas (por exemplo `coleta.endereco`)."
                ),
                caminho,
            );
            return;
        }
        let ir = self.ir;
        let Some(indice) = self.etapas.get(etapa_id).copied() else {
            self.erro_no(
                Code::UnknownField,
                format!("{onde} compara `{campo}`, mas a etapa `{etapa_id}` não existe."),
                caminho,
            );
            return;
        };
        let Some(campo_info) = ir.stages[indice].campos.iter().find(|c| c.id == campo_id) else {
            self.erro_no(
                Code::UnknownField,
                format!(
                    "{onde} compara `{campo}`, mas a etapa `{etapa_id}` não tem o campo `{campo_id}`."
                ),
                caminho,
            );
            return;
        };
        let (tipo_campo, opcoes) = (campo_info.tipo, campo_info.options.clone());
        self.lidos
            .entry(ir.stages[indice].id.as_str())
            .or_default()
            .insert(campo_info.id.as_str());

        match tipo {
            "igual" | "diferente" => {
                if let Some(valor) = valor
                    && !opcoes.is_empty()
                    && !opcoes.iter().any(|o| o == valor)
                {
                    self.aviso_no(
                        Code::OutsideOptions,
                        format!(
                            "{onde} compara `{campo}` com `{valor}`, que não é uma das opções ({}): o teste nunca casa.",
                            opcoes.join(", ")
                        ),
                        caminho,
                    );
                }
            }
            "maior" | "menor" if tipo_campo != FieldKind::Number => {
                self.aviso_no(
                    Code::TextComparison,
                    format!(
                        "{onde} usa `{tipo}` com `{campo}`, mas o campo é `{}`: mude o tipo para `numero` ou use `igual`.",
                        tipo_campo.as_text()
                    ),
                    caminho,
                );
            }
            _ => {}
        }
    }

    // -------------------------------------------------------------- destinos e fins

    fn confere_destino(&mut self, destino: &Destination, caminho: &str, onde: &str) {
        match destino {
            Destination::Etapa { id } => {
                if !self.etapas.contains_key(id.as_str()) {
                    self.erro_no(
                        Code::UnknownDestination,
                        format!(
                            "{onde} vai para a etapa `{id}`, que não existe. Use o id de uma etapa ou de uma decisão."
                        ),
                        caminho,
                    );
                }
            }
            Destination::Decisao { id } => {
                if !self.decisoes.contains_key(id.as_str()) {
                    self.erro_no(
                        Code::UnknownDestination,
                        format!(
                            "{onde} vai para a decisão `{id}`, que não existe. Use o id de uma etapa ou de uma decisão."
                        ),
                        caminho,
                    );
                }
            }
            Destination::Fim {
                id,
                resultado,
                ends_service: encerra_atendimento,
            } => match self.fins.get(id.as_str()).copied() {
                None => self.erro_no(
                    Code::UnknownDestination,
                    format!(
                        "{onde} termina no fim `{id}`, que não está declarado em `fins` deste fluxo."
                    ),
                    caminho,
                ),
                Some(indice) => {
                    let fim = &self.ir.ends[indice];
                    if fim.resultado != *resultado
                        || fim.ends_service != *encerra_atendimento
                    {
                        self.erro_no(
                            Code::DivergentEnd,
                            format!(
                                "O fim `{id}` diz uma coisa aqui e outra em `fins[{indice}]` ({}): use a mesma declaração de `fim(...)` nos dois lugares.",
                                if fim.resultado != *resultado {
                                    "o resultado é diferente"
                                } else {
                                    "o `encerra_atendimento` é diferente"
                                }
                            ),
                            caminho.to_owned(),
                        );
                    }
                }
            },
        }
    }

    fn confere_fins(&mut self) {
        let ir = self.ir;
        let mut vistos: BTreeSet<&str> = BTreeSet::new();
        for (i, fim) in ir.ends.iter().enumerate() {
            if !vistos.insert(fim.id.as_str()) {
                self.erro_no(
                    Code::DuplicateId,
                    format!(
                        "O fim `{}` está declarado duas vezes em `fins`; use a mesma declaração nos dois lugares.",
                        fim.id
                    ),
                    format!("fins[{i}] (id \"{}\")", fim.id),
                );
            }
        }
    }

    fn confere_inicio(&mut self) {
        if self.ir.start.is_empty() {
            self.erro_no(
                Code::UnknownDestination,
                "O fluxo não declara a etapa de início. Indique o id da etapa em que o atendimento começa.",
                "inicio",
            );
        } else if !self.etapas.contains_key(self.ir.start.as_str()) {
            self.erro_no(
                Code::UnknownDestination,
                format!(
                    "O fluxo começa em `{}`, que não é uma etapa deste fluxo. Use o id de uma etapa.",
                    self.ir.start
                ),
                "inicio",
            );
        }
    }

    // -------------------------------------------------------------- grafo

    fn confere_ciclo(&mut self) {
        let ir = self.ir;
        let mut grafo: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for decisao in &ir.decisions {
            let mut arestas: Vec<&str> = Vec::new();
            for destino in [&decisao.then_branch, &decisao.else_branch] {
                if let Destination::Decisao { id } = destino {
                    arestas.push(id.as_str());
                }
            }
            grafo.insert(decisao.id.as_str(), arestas);
        }
        let Some(ciclo) = procura_ciclo(&grafo) else {
            return;
        };
        let primeiro = ciclo.first().map(String::as_str).unwrap_or("");
        let indice = self.decisoes.get(primeiro).copied().unwrap_or(0);
        let caminho = ir
            .decisions
            .get(indice)
            .map(|decisao| caminho_decisao(indice, decisao))
            .unwrap_or_else(|| "decisoes".to_owned());
        self.erro_no(
            Code::DecisionCycle,
            format!(
                "As decisões formam um ciclo ({}): um fluxo não pode voltar a uma decisão já avaliada.",
                ciclo.join(" → ")
            ),
            caminho,
        );
    }

    fn confere_alcance(&mut self) {
        let ir = self.ir;
        let inicio = ir.start.as_str();
        if inicio.is_empty() || !self.etapas.contains_key(inicio) {
            // Sem início válido não há de onde alcançar; o `FLX15` do início já explicou.
            return;
        }
        let mut grafo: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for etapa in &ir.stages {
            let arestas = grafo.entry(etapa.id.as_str()).or_default();
            for acao in &etapa.actions {
                if let Some(id) = acao.goes_to.is_open() {
                    arestas.push(id);
                }
            }
        }
        for decisao in &ir.decisions {
            let arestas = grafo.entry(decisao.id.as_str()).or_default();
            for destino in [&decisao.then_branch, &decisao.else_branch] {
                if let Some(id) = destino.is_open() {
                    arestas.push(id);
                }
            }
        }

        let mut alcancados: BTreeSet<&str> = BTreeSet::new();
        let mut pilha = vec![inicio];
        while let Some(no) = pilha.pop() {
            if !alcancados.insert(no) {
                continue;
            }
            if let Some(destinos) = grafo.get(no) {
                for destino in destinos {
                    pilha.push(destino);
                }
            }
        }

        for (i, etapa) in ir.stages.iter().enumerate() {
            if !alcancados.contains(etapa.id.as_str()) {
                self.aviso_no(
                    Code::Unreachable,
                    format!(
                        "A etapa `{}` não é alcançável a partir de `inicio = \"{inicio}\"`. Confira os `vai_para`.",
                        etapa.id
                    ),
                    caminho_etapa(i, etapa),
                );
            }
        }
        for (i, decisao) in ir.decisions.iter().enumerate() {
            if !alcancados.contains(decisao.id.as_str()) {
                self.aviso_no(
                    Code::Unreachable,
                    format!(
                        "A decisão `{}` não é alcançável a partir de `inicio = \"{inicio}\"`. Confira os `vai_para` e os ramos `entao`/`senao`.",
                        decisao.id
                    ),
                    caminho_decisao(i, decisao),
                );
            }
        }
    }

    fn confere_teto(&mut self) {
        let nos = self.ir.stages.len() + self.ir.decisions.len() + self.ir.ends.len();
        if nos > MAX_NOS {
            self.erro_no(
                Code::NodeLimit,
                format!(
                    "O fluxo tem {nos} nós e o limite é {MAX_NOS}. Junte etapas parecidas ou divida o fluxo."
                ),
                format!("fluxo (id \"{}\")", self.ir.fluxo.id),
            );
        }
    }

    // -------------------------------------------------------------- uso

    fn confere_uso_dos_campos(&mut self) {
        let ir = self.ir;
        for (i, etapa) in ir.stages.iter().enumerate() {
            let base = caminho_etapa(i, etapa);
            for (j, campo) in etapa.campos.iter().enumerate() {
                let exigido = self
                    .exigidos
                    .get(etapa.id.as_str())
                    .is_some_and(|exigidos| exigidos.contains(campo.id.as_str()));
                let lido = self
                    .lidos
                    .get(etapa.id.as_str())
                    .is_some_and(|lidos| lidos.contains(campo.id.as_str()));
                if exigido || lido {
                    continue;
                }
                self.aviso_no(
                    Code::UnusedField,
                    format!(
                        "O campo `{}.{}` não é exigido por nenhuma ação nem lido por nenhuma regra: ninguém usa o valor.",
                        etapa.id, campo.id
                    ),
                    sub(&base, &format!("campos[{j}]")),
                );
            }
        }
    }

    fn confere_uso_das_defs(&mut self) {
        let ir = self.ir;
        let defs: Vec<(&str, usize)> = ir
            .defs
            .iter()
            .enumerate()
            .map(|(i, def)| (def.nome.as_str(), i))
            .collect();
        for (nome, indice) in defs {
            if self.usadas.contains(nome) {
                continue;
            }
            self.aviso_no(
                Code::UnusedDef,
                format!(
                    "A função `{nome}` está definida e nenhuma decisão usa `funcao(\"{nome}\")`. Chame ou apague."
                ),
                format!("defs[{indice}] (nome \"{nome}\")"),
            );
        }
    }
}

/// Chave de comparação de uma folha de combinador: campo, valor e o nome do combinador.
fn chave_da_folha(regra: &Rule) -> Option<(&str, Option<&str>, &'static str)> {
    match regra {
        Rule::Igual {
            campo,
            value: valor,
        } => Some((campo, Some(valor), "igual")),
        Rule::Diferente {
            campo,
            value: valor,
        } => Some((campo, Some(valor), "diferente")),
        Rule::Maior {
            campo,
            value: valor,
        } => Some((campo, Some(valor), "maior")),
        Rule::Menor {
            campo,
            value: valor,
        } => Some((campo, Some(valor), "menor")),
        Rule::Contem {
            campo,
            value: valor,
        } => Some((campo, Some(valor), "contem")),
        Rule::Vazio { campo } => Some((campo, None, "vazio")),
        Rule::E { .. } | Rule::Ou { .. } | Rule::Nao { .. } | Rule::Funcao { .. } => None,
    }
}

/// Se a lista tem um valor repetido (comparação exata).
fn repetidas(valores: &[String]) -> bool {
    let mut vistos: BTreeSet<&str> = BTreeSet::new();
    valores.iter().any(|valor| !vistos.insert(valor.as_str()))
}

/// Primeiro ciclo do grafo de decisões, se houver: caminho com o nó repetido no fim (`a → b → a`).
///
/// Percorre os nós na ordem dos ids (determinístico), como o compilador faz.
fn procura_ciclo(grafo: &BTreeMap<&str, Vec<&str>>) -> Option<Vec<String>> {
    fn visita(
        no: &str,
        grafo: &BTreeMap<&str, Vec<&str>>,
        estado: &mut BTreeMap<String, u8>,
        caminho: &mut Vec<String>,
        achado: &mut Option<Vec<String>>,
    ) {
        if achado.is_some() || !grafo.contains_key(no) {
            return;
        }
        match estado.get(no).copied().unwrap_or(0) {
            1 => {
                let mut ciclo: Vec<String> = caminho
                    .iter()
                    .skip_while(|atual| atual.as_str() != no)
                    .cloned()
                    .collect();
                ciclo.push(no.to_owned());
                *achado = Some(ciclo);
                return;
            }
            2 => return,
            _ => {}
        }
        estado.insert(no.to_owned(), 1);
        caminho.push(no.to_owned());
        if let Some(destinos) = grafo.get(no) {
            for destino in destinos {
                visita(destino, grafo, estado, caminho, achado);
            }
        }
        caminho.pop();
        estado.insert(no.to_owned(), 2);
    }

    let mut estado: BTreeMap<String, u8> = BTreeMap::new();
    let mut caminho: Vec<String> = Vec::new();
    let mut achado: Option<Vec<String>> = None;
    for raiz in grafo.keys() {
        if achado.is_some() {
            break;
        }
        visita(raiz, grafo, &mut estado, &mut caminho, &mut achado);
    }
    achado
}
