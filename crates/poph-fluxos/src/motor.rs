//! Motor de execução de fluxos: iniciar, registrar campos, executar ação, atravessar decisões,
//! concluir num `fim(...)`, abandonar e descrever os efeitos (FX-13 e FX-14).
//!
//! As funções aqui são **puras**: recebem a estrutura compilada (a IR da revisão publicada) e o
//! estado da execução que veio do banco, e devolvem o estado novo com os eventos que a operação
//! atômica do `poph-store` vai gravar (FX-20). Não há banco, relógio, rede nem `async`: o tempo e a
//! `version` são de quem chama, e o teste roda a mesma entrada quantas vezes quiser com o mesmo
//! resultado.
//!
//! # O que esta tarefa faz e o que não faz
//!
//! - **Faz:** [`iniciar`] (execução nova na etapa de `inicio`), [`executar_acao`] (confere ator,
//!   campos e o critério `auto`; grava os campos; avança a execução quando a etapa terminou),
//!   [`abandonar`], a travessia de **decisões encadeadas** até um nó de etapa ou de fim, a
//!   conclusão num `fim(...)` com o pedido de encerramento quando o fim pede, a produção dos
//!   **efeitos** com o texto resolvido e a chave de idempotência, e as consultas que o prompt e o
//!   banco precisam ([`atores_exigidos`], [`campos_faltando`], [`dados_da_etapa`],
//!   [`efeitos_de_entrada`]).
//! - **Não faz (FX-20):** transação, `version`, isolamento entre Ficha e fluxo, escrita do evento,
//!   fechar conversa ou Ficha. O motor descreve; o store grava. Em particular, chegar a um
//!   `fim(encerra_atendimento=True)` só **pede** o encerramento ([`Resultado::encerra_atendimento`]):
//!   quem fecha é o chamador, na mesma transação.
//! - **Não faz (FX-21):** entregar efeito. O motor só descreve o que fazer ([`EfeitoProduzido`]),
//!   com a chave que torna a criação idempotente; a fila e as tentativas são do store.
//!
//! # Decisão, limite e ciclo
//!
//! Uma decisão é avaliada **a partir dos dados já registrados** ([`crate::regras`]) e segue o ramo
//! na mesma passagem: não existe ação intermediária automática nem busca em largura. Cada decisão
//! resolvida vira um evento `decisao_resolvida` com o ramo escolhido.
//!
//! - Até [`MAX_SALTOS_DECISAO`] decisões encadeadas (ou o teto de
//!   [`Contexto::max_saltos_decisao`]): a execução segue normalmente.
//! - Ciclo de decisões (a mesma decisão duas vezes na mesma passagem) ou estouro do teto: a
//!   execução fica **`falha`** com o motivo em [`Estado::motivo`] e no evento `execucao_falhou`. Não
//!   é erro da chamada: o fluxo publicado é que está defeituoso (o validador recusa o ciclo com
//!   `FLX23` na publicação), e a Ficha continua aberta.
//! - Regra que não pôde ser avaliada (`regra_invalida`, `regra_limite`): mesmo desfecho — execução
//!   `falha`, motivo gravado, Ficha aberta (`FLXR01`/`FLXR03`).
//! - Nó de destino que não existe na estrutura (`etapa` ou `decisao`): [`DomainError::IrDivergente`].
//!   A IR gravada diverge do que o motor sabe conduzir e o motor falha fechado, sem chutar.
//!
//! # Efeitos
//!
//! Os efeitos declarados em `acoes[].efeitos` (disparados depois de a transição ser aceita) e em
//! `etapas[].ao_entrar` (ao entrar na etapa, antes de qualquer campo dela) saem em
//! [`Resultado::efeitos`], na ordem declarada, já com o texto resolvido e com a **chave** que os
//! torna idempotentes: `nó + índice` (`acao:<etapa>.<acao>/0`, `etapa:<id>/0`). A execução entra na
//! chave pelo índice único do banco (`org_id, execucao_id, chave`), então o motor não precisa do id
//! da execução para descrever o efeito.
//!
//! Interpolação: `{etapa.campo}` vira o texto canônico do valor (`FLUXOS_LINGUAGEM.md` §5),
//! `{ficha.id}` e `{cliente.peer}` vêm do [`Contexto`], e **variável que não existe vira texto
//! vazio** (`FLX59` avisa disso na publicação). O `para` do `notificar_whatsapp` e o nome do
//! departamento do `encaminhar` são identificadores, não texto: não são interpolados.
//!
//! # Critério `auto` do avanço (PRD §6.4, paridade com `state_engine.py`)
//!
//! A etapa avança quando **todo ator que tem uma ação própria restrita que leva para frente já
//! atuou**. A conta é derivada das **ações**, nunca da lista `atores` da etapa:
//!
//! | Situação na etapa | Efeito |
//! |---|---|
//! | ação restrita a `atendente` que vai para uma etapa ou decisão | exige o `atendente` |
//! | ação sem restrição de ator (ou restrita a quem está agindo) | não exige ninguém: a rota é a dela |
//! | ação que vai para `fim(...)` | não exige ninguém **e** é decisiva: leva ao desfecho mesmo com outro ator pendente |
//! | ator que só aparece em `atores` da etapa, sem ação dele | não trava a etapa (`FLX62` na publicação) |
//! | cliente | nunca é ator de ação (D5); não existe no cálculo |
//!
//! Enquanto faltar alguém, a etapa fica `parcial` e [`Resultado::aguardando`] diz **quem falta** —
//! é o texto que a ferramenta devolve ao agente. Duas ações restritas (uma do agente, outra do
//! atendente) chegam ao destino de **quem agiu por último**, em qualquer ordem.
//!
//! "Não exige ninguém" é sobre a ação, não sobre a etapa: se a **outra** ação da etapa é restrita e
//! o dono dela ainda não agiu, a ação irrestrita também espera (é o que `state_engine.py` faz — o
//! `resolve_auto_advance` olha as ações *que não foram executadas*). Sem isso, duas rotas
//! concorrentes da mesma etapa avançariam em ordens diferentes.
//!
//! # Sem transição escondida
//!
//! Ação que não existe na etapa devolve [`DomainError::AcaoInvalida`] com a **lista das válidas** e
//! não muda nada: o motor não procura em largura nem executa ação intermediária (é o que o POP
//! Atende fazia e o PRD §8 remove).
//!
//! # Idempotência
//!
//! Toda mutação aceita uma `chave_idempotencia`. O estado guarda as chaves já aplicadas com o que
//! cada uma produziu ([`Recibo`]): repetir a chave devolve `repetida = true`, o estado como está e
//! **nenhum evento** novo. No banco, a garantia é o índice único de `ficha_eventos.chave_idempotencia`
//! (PRD §7.1); aqui a chave fica no estado para a função ser pura e o teste provar a repetição.
//!
//! # Tipo de campo e dado do cliente
//!
//! O valor entra **como valor**: nunca é interpretado como código (as regras também não o fazem, §6
//! do documento). O tipo declarado é conferido de forma estrita, igual ao POP Atende
//! (`validators.py`): `numero` exige número JSON, `booleano` exige booleano JSON, `opcao` exige um
//! dos ids declarados. Texto é gravado sem os espaços das pontas (§5). `null` vale como "não
//! preenchido" no campo opcional e é recusado no obrigatório.
//!
//! Um detalhe de propósito: `exige` é conferido **depois** de gravar os campos desta chamada e
//! junto do que a etapa já tinha. O agente pode ter informado o endereço na chamada anterior e só
//! agora pedir a ação que o exige; recusar por isso faria o modelo repetir o dado.
//!
//! # Limites conhecidos desta tarefa
//!
//! - `data` confere forma e faixas do ISO 8601 (mês 1–12, dia 1–31, hora/minuto/segundo), **não** o
//!   calendário: `2026-02-30` passa. Calendário completo é de quem consome a data.
//! - `dados[etapa]` que não seja objeto é tratado como etapa sem campo nenhum (não é erro: o motor
//!   não conserta nem acusa dado que não é dele).
//! - A chave do efeito é `nó + índice`, sem o id da execução: duas visitas à **mesma** etapa (um
//!   fluxo que volta) geram a mesma chave e o `ao_entrar` dela não é enfileirado duas vezes. É o
//!   que a especificação pede (uma mensagem por efeito), mas quem escrever um laço de etapas com
//!   `mensagem_ao_cliente` no `ao_entrar` recebe o texto do cliente uma vez só.
//! - Eleger o atendente para abandonar é do chamador: o motor registra quem abandonou, não confere
//!   permissão.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json, json};

use poph_types::{
    AtorAcao, AtorEvento, DomainError, DomainResult, StatusExecucao, TipoCampo, TipoEventoFicha,
};

use crate::ir::{Campo, Decisao, Destino, Efeito, Etapa, Ir};
use crate::regras::{Avaliador, Dados, preenchido, texto_canonico};

/// Valores de campo informados junto de uma ação: `campo -> valor`.
///
/// As chaves são ids de campo **da etapa atual** (`campos[].id`), sem o prefixo da etapa. O que não
/// for campo da etapa é recusado com a lista dos campos dela.
pub type Campos = Map<String, Json>;

/// Teto de decisões encadeadas numa mesma passagem (`fluxos.max_saltos_decisao`, PRD §9).
pub const MAX_SALTOS_DECISAO: usize = 10;

/// O que o motor não guarda no estado da execução e o chamador empresta.
///
/// São três coisas de fora: a **Ficha** e o **contato do cliente** (a execução não é dona deles, e
/// os efeitos os citam em `{ficha.id}` e `{cliente.peer}`) e o **avaliador das regras** da revisão
/// publicada (é ele que resolve `funcao("nome")`, com o módulo já carregado uma vez).
///
/// O teto de saltos vem de `fluxos.max_saltos_decisao`; o padrão é [`MAX_SALTOS_DECISAO`].
#[derive(Debug, Clone, Copy)]
pub struct Contexto<'a> {
    /// Id da Ficha do atendimento (o que `{ficha.id}` resolve).
    pub ficha_id: &'a str,
    /// Contato do cliente na conversa (o que `{cliente.peer}` resolve).
    pub cliente_peer: &'a str,
    /// Avaliador das regras da revisão em uso.
    pub avaliador: &'a Avaliador,
    /// Teto de decisões encadeadas na mesma passagem.
    pub max_saltos_decisao: usize,
}

impl<'a> Contexto<'a> {
    /// Contexto mínimo: sem Ficha, sem cliente e sem `def` no avaliador.
    pub fn novo(avaliador: &'a Avaliador) -> Self {
        Self {
            ficha_id: "",
            cliente_peer: "",
            avaliador,
            max_saltos_decisao: MAX_SALTOS_DECISAO,
        }
    }

    /// A Ficha do atendimento (para `{ficha.id}`).
    pub fn com_ficha(mut self, ficha_id: &'a str) -> Self {
        self.ficha_id = ficha_id;
        self
    }

    /// O contato do cliente (para `{cliente.peer}`).
    pub fn com_cliente(mut self, cliente_peer: &'a str) -> Self {
        self.cliente_peer = cliente_peer;
        self
    }

    /// O teto de decisões encadeadas (`fluxos.max_saltos_decisao`); o teste aperta para provar que
    /// está ligado.
    pub fn com_limite_de_saltos(mut self, limite: usize) -> Self {
        self.max_saltos_decisao = limite;
        self
    }
}

/// Estado de uma execução de fluxo na forma **pura** que o motor lê e devolve.
///
/// Espelha `fluxo_execucoes` (PRD §7.1) sem o que só o banco sabe: os ids UUID, a `version`, a Ficha
/// e os carimbos de tempo ficam no `poph-store` (FX-20).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Estado {
    /// Slug do fluxo (`fluxo.id` da IR). O id de banco do fluxo é do store.
    pub fluxo_id: String,
    /// Situação da execução; o motor só aceita ação em [`StatusExecucao::Ativa`].
    pub status: StatusExecucao,
    /// Nó em que a execução está: etapa, decisão aguardando resolução (FX-14) ou fim (FX-14).
    pub no_atual: Destino,
    /// `dados[etapa][campo]` — o mesmo formato de `fluxo_execucoes.dados_json`.
    pub dados: Dados,
    /// Atores que já agiram **na etapa atual**, em ordem alfabética do texto (`agente`,
    /// `atendente`); zera ao entrar numa etapa nova.
    pub atores_que_agiram: Vec<AtorAcao>,
    /// Resumo do desfecho (`fins[].resultado`), preenchido quando a execução chega ao fim.
    pub resultado: Option<String>,
    /// Motivo do abandono ou da falha (`fluxo_execucoes.motivo_fim`); `None` no caminho feliz.
    pub motivo: Option<String>,
    /// Chaves de idempotência já aplicadas, com o desfecho de cada uma.
    pub chaves: BTreeMap<String, Recibo>,
}

/// O que uma chave de idempotência produziu, para a repetição devolver o mesmo desfecho.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recibo {
    /// Nó em que a execução ficou naquela passagem.
    pub no_atual: Destino,
    /// Quem ainda faltava agir quando a etapa ficou `parcial`; vazio quando a etapa avançou.
    pub aguardando: Vec<AtorAcao>,
}

/// Um evento que a operação atômica do store grava em `ficha_eventos`.
///
/// O motor não inventa `seq` nem id: isso é da Ficha (FX-20). O `payload` é o JSON do evento, já
/// estável para o teste comparar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evento {
    /// Tipo do evento (`ficha_eventos.tipo`).
    pub tipo: TipoEventoFicha,
    /// Quem assinou (`ficha_eventos.ator_tipo`): o ator que agiu ou o sistema.
    pub ator: AtorEvento,
    /// Conteúdo do evento.
    pub payload: Json,
}

/// Desfecho de uma passagem do motor: o estado novo, o que gravar e o que dizer ao agente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resultado {
    /// Estado da execução depois da passagem (inalterado quando `repetida`).
    pub novo_estado: Estado,
    /// Eventos a gravar, na ordem em que aconteceram: campos, ação, efeitos, etapa concluída,
    /// decisões resolvidas, etapa entrou ou execução concluída.
    pub eventos: Vec<Evento>,
    /// Efeitos a enfileirar, na ordem declarada e com o texto já resolvido.
    pub efeitos: Vec<EfeitoProduzido>,
    /// Atores que ainda precisam agir porque a etapa ficou `parcial`; vazio quando avançou.
    pub aguardando: Vec<AtorAcao>,
    /// `true` quando a `chave_idempotencia` já tinha sido aplicada: nada mudou e nada é gravado.
    pub repetida: bool,
    /// `true` quando a execução chegou a um `fim(encerra_atendimento=True)`: o chamador **pede** o
    /// encerramento do atendimento (fechar conversa e Ficha é dele, na mesma transação).
    ///
    /// Sai `false` quando `repetida`: repetir a chave não pede nada de novo.
    pub encerra_atendimento: bool,
    /// `true` quando esta passagem parou a execução em `falha` porque uma **regra não avaliou**
    /// (`FLXR01`/`FLXR03`, o motivo em [`Estado::motivo`]).
    ///
    /// Ciclo de decisões e estouro de `fluxos.max_saltos_decisao` também deixam a execução `falha`,
    /// mas não são erro de regra: aqui só entram as avaliações que falharam. Serve ao contador
    /// `poph_fluxos_erros_regra_total` (FX-40): o espelho do banco não sabe distinguir a causa pelo
    /// texto do motivo, então ela sobe tipada daqui.
    pub erro_de_regra: bool,
}

/// Efeito declarado na IR, já com o texto resolvido e com a chave que o torna idempotente.
///
/// É o que a fila de efeitos (`fluxo_efeitos`, FX-21) grava: a **chave** é a identidade do efeito, e
/// o `efeito` é o que fazer. O motor só descreve; entregar é da fila.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EfeitoProduzido {
    /// Chave do efeito: `nó + índice` (`etapa:coleta/0`, `acao:coleta.confirmar/1`).
    ///
    /// A execução entra na chave pelo índice único do banco (`org_id, execucao_id, chave`), então a
    /// mesma transição refeita não gera uma segunda mensagem.
    pub chave: String,
    /// Nó que declarou o efeito: `etapa:<id>` (`ao_entrar`) ou `acao:<etapa>.<ação>`.
    pub no: String,
    /// Posição do efeito na lista declarada (base zero).
    pub indice: usize,
    /// O que fazer, com o texto já interpolado.
    pub efeito: Efeito,
}

impl EfeitoProduzido {
    /// Tipo do efeito como a coluna `fluxo_efeitos.tipo` grava.
    pub fn tipo(&self) -> &'static str {
        match self.efeito {
            Efeito::MensagemAoCliente { .. } => "mensagem_ao_cliente",
            Efeito::NotificarWhatsapp { .. } => "notificar_whatsapp",
            Efeito::Encaminhar { .. } => "encaminhar",
        }
    }
}

/// Cria a execução na etapa de `inicio`, com os dados vazios.
///
/// Devolve erro quando a IR aponta para uma etapa que não existe (`ir_divergente`): a validação da
/// FX-11 já recusa isso na publicação, mas uma IR gravada à mão não pode fazer o motor entrar em
/// pânico.
pub fn iniciar(ir: &Ir) -> DomainResult<Estado> {
    let etapa = etapa_por_id(ir, &ir.inicio)?;
    Ok(Estado {
        fluxo_id: ir.fluxo.id.clone(),
        status: StatusExecucao::Ativa,
        no_atual: Destino::Etapa {
            id: etapa.id.clone(),
        },
        dados: Dados::new(),
        atores_que_agiram: Vec::new(),
        resultado: None,
        motivo: None,
        chaves: BTreeMap::new(),
    })
}

/// Executa uma ação da etapa atual: confere ator, campos e o critério `auto`, grava os campos,
/// atravessa as decisões e conclui quando o destino é um `fim(...)`.
///
/// A ordem das conferências é a do POP Atende (`state_engine.execute_action`), com a mensagem que o
/// agente precisa para se corrigir:
///
/// 1. execução `ativa`;
/// 2. chave de idempotência já aplicada → devolve o mesmo desfecho, sem evento e sem efeito;
/// 3. execução parada numa decisão (estado gravado por uma versão anterior) → resolve antes;
/// 4. o nó atual é uma etapa (fim não aceita ação);
/// 5. a ação existe na etapa (senão erro com a **lista das válidas**);
/// 6. o ator age nesta etapa (`atores_acao`);
/// 7. a ação não é restrita a outro ator;
/// 8. cada campo informado existe na etapa e tem o tipo/opção certo;
/// 9. todo campo de `exige` está preenchido;
/// 10. então grava os campos, decide o avanço, produz os efeitos e segue o destino.
///
/// Nada disso grava pela metade: qualquer recusa deixa o estado como estava (o chamador fica com o
/// `estado` que passou). A exceção é o fluxo **defeituoso** (ciclo de decisões, estouro de saltos,
/// regra que não avalia): aí a passagem volta `Ok` com a execução marcada `falha`, porque o motor
/// conduziu o estado até onde dava e o chamador precisa gravar o desfecho.
pub fn executar_acao(
    ir: &Ir,
    estado: &Estado,
    acao: &str,
    campos: &Campos,
    ator: AtorAcao,
    chave_idempotencia: Option<&str>,
    contexto: &Contexto<'_>,
) -> DomainResult<Resultado> {
    // 1. Execução que não está ativa não aceita ação (concluída, abandonada ou em falha).
    if estado.status != StatusExecucao::Ativa {
        return Err(recusa(
            acao,
            format!(
                "a execução está {} e não aceita ação; comece outro fluxo ou leia o estado",
                estado.status
            ),
        ));
    }

    // 2. Chave já aplicada: devolve o desfecho anterior e não grava nada de novo.
    if let Some(chave) = chave_idempotencia
        && let Some(recibo) = estado.chaves.get(chave)
    {
        return Ok(Resultado {
            novo_estado: estado.clone(),
            eventos: Vec::new(),
            efeitos: Vec::new(),
            aguardando: recibo.aguardando.clone(),
            repetida: true,
            encerra_atendimento: false,
            erro_de_regra: false,
        });
    }

    let mut trabalho = estado.clone();
    let mut eventos: Vec<Evento> = Vec::new();
    let mut efeitos: Vec<EfeitoProduzido> = Vec::new();

    // 3. Estado parado numa decisão: resolve antes de olhar a ação. Desde esta tarefa o motor
    //    atravessa a decisão na mesma passagem, então isso só aparece em estado gravado por uma
    //    versão anterior do motor.
    if let Destino::Decisao { .. } = trabalho.no_atual {
        let parada = trabalho.no_atual.clone();
        let desfecho = seguir(
            ir,
            &mut trabalho,
            parada,
            contexto,
            &mut eventos,
            &mut efeitos,
        )?;
        if trabalho.status != StatusExecucao::Ativa {
            return Ok(resultado_parado(
                trabalho,
                eventos,
                efeitos,
                desfecho.erro_de_regra,
            ));
        }
    }

    // 4. A execução precisa estar numa etapa para receber ação.
    let etapa_id = match &trabalho.no_atual {
        Destino::Etapa { id } => id.clone(),
        Destino::Decisao { id } => {
            // Inalcançável: a decisão acima foi resolvida ou a execução parou.
            return Err(recusa(
                acao,
                format!(
                    "a execução está na decisão `{id}`: ela precisa ser resolvida antes da ação"
                ),
            ));
        }
        Destino::Fim { id, .. } => {
            return Err(recusa(
                acao,
                format!("a execução chegou ao fim `{id}`: não há ação depois dele"),
            ));
        }
    };
    let etapa = etapa_por_id(ir, &etapa_id)?;

    // 5. A ação existe nesta etapa (sem busca em largura: só as ações daqui).
    let Some(acao_ir) = etapa.acoes.iter().find(|candidata| candidata.id == acao) else {
        return Err(recusa(
            acao,
            format!(
                "a etapa `{}` não tem essa ação; válidas: {}",
                etapa.id,
                lista_de_acoes(etapa)
            ),
        ));
    };

    // 6. O ator age nesta etapa.
    if !etapa.atores_acao.contains(&ator) {
        return Err(recusa(
            acao,
            format!(
                "o ator `{ator}` não age na etapa `{}`; atores: {}",
                etapa.id,
                lista_de_atores(&etapa.atores_acao)
            ),
        ));
    }

    // 7. A ação não é de outro ator.
    if let Some(dono) = acao_ir.ator
        && dono != ator
    {
        return Err(recusa(
            acao,
            format!("a ação `{acao}` é restrita ao ator `{dono}`"),
        ));
    }

    // 8. Campos: existem na etapa, tipo certo, opção da lista. Nada disso grava pela metade.
    let validados = match validar_campos(etapa, campos) {
        Ok(validados) => validados,
        Err(erros) => return Err(campos_invalidos(etapa, erros)),
    };

    // 9. `exige`: os campos da própria etapa, contando o que já estava gravado.
    let mut registrados = dados_da_etapa(&trabalho.dados, &etapa.id);
    registrados.extend(validados.clone());
    let mut faltando: Vec<String> = acao_ir
        .exige
        .iter()
        .filter(|campo| !preenchido(registrados.get(*campo)))
        .map(|campo| {
            if etapa.campos.iter().any(|declarado| declarado.id == *campo) {
                format!("a ação `{acao}` exige o campo `{campo}`, que ainda não foi preenchido")
            } else {
                // IR de fora ou revisão antiga: `exige` aponta para campo que a etapa não declara
                // (o validador recusa isto com `FLX17` na publicação).
                format!(
                    "a ação `{acao}` exige o campo `{campo}`, que a etapa `{}` não tem",
                    etapa.id
                )
            }
        })
        .collect();
    if !faltando.is_empty() {
        faltando.sort();
        return Err(campos_invalidos(etapa, faltando));
    }

    // 10. Critério `auto`: quem falta agir para a etapa avançar.
    let mut progresso = trabalho.atores_que_agiram.clone();
    if !progresso.contains(&ator) {
        progresso.push(ator);
    }
    ordena(&mut progresso);

    let mut aguardando: Vec<AtorAcao> = atores_exigidos(etapa)
        .into_iter()
        .filter(|exigido| !progresso.contains(exigido))
        .collect();
    ordena(&mut aguardando);

    // Ação de saída (`fim`) é decisiva: leva ao desfecho mesmo com outro ator pendente, como no
    // `state_engine.py` ("encerramento é decisivo").
    let decisiva = matches!(acao_ir.vai_para, Destino::Fim { .. });
    let avanca = decisiva || aguardando.is_empty();

    // 11. Eventos e estado novo.
    let autor = ator_de_evento(ator);
    if !validados.is_empty() {
        eventos.push(Evento {
            tipo: TipoEventoFicha::CamposPreenchidos,
            ator: autor,
            payload: json!({ "etapa": etapa.id, "campos": validados }),
        });
    }
    eventos.push(Evento {
        tipo: TipoEventoFicha::AcaoExecutada,
        ator: autor,
        payload: json!({
            "etapa": etapa.id,
            "acao": acao,
            "destino": acao_ir.vai_para,
            "parcial": !avanca,
            "aguardando": aguardando,
        }),
    });

    let mut novo_estado = trabalho;
    novo_estado
        .dados
        .insert(etapa.id.clone(), Json::Object(registrados));

    let mut encerra_atendimento = false;
    let mut erro_de_regra = false;
    let aguardando_do_resultado = if avanca {
        // Efeitos da ação: saem depois de a transição ser aceita (`FLUXOS_LINGUAGEM.md` §4.9).
        let no_da_acao = format!("acao:{}.{acao}", etapa.id);
        for efeito in efeitos_declarados(&acao_ir.efeitos, no_da_acao, &novo_estado.dados, contexto)
        {
            eventos.push(evento_do_efeito(&efeito));
            efeitos.push(efeito);
        }
        eventos.push(Evento {
            tipo: TipoEventoFicha::EtapaConcluida,
            ator: AtorEvento::Sistema,
            payload: json!({ "etapa": etapa.id, "acao": acao }),
        });
        novo_estado.atores_que_agiram = Vec::new();
        let desfecho = seguir(
            ir,
            &mut novo_estado,
            acao_ir.vai_para.clone(),
            contexto,
            &mut eventos,
            &mut efeitos,
        )?;
        encerra_atendimento = desfecho.encerra_atendimento;
        erro_de_regra = desfecho.erro_de_regra;
        Vec::new()
    } else {
        novo_estado.atores_que_agiram = progresso;
        aguardando
    };

    if let Some(chave) = chave_idempotencia {
        novo_estado.chaves.insert(
            chave.to_owned(),
            Recibo {
                no_atual: novo_estado.no_atual.clone(),
                aguardando: aguardando_do_resultado.clone(),
            },
        );
    }

    Ok(Resultado {
        novo_estado,
        eventos,
        efeitos,
        aguardando: aguardando_do_resultado,
        repetida: false,
        encerra_atendimento,
        erro_de_regra,
    })
}

/// Abandona a execução: ela para onde está, com o motivo gravado, e **não** produz efeito nenhum.
///
/// Quem pode abandonar (o agente pela ferramenta `fluxo_abandonar`, o atendente pela API) é do
/// chamador; o motor registra o ator no evento e não confere permissão. O estado guarda o motivo,
/// que é o mesmo texto que a coluna `fluxo_execucoes.motivo_fim` recebe.
///
/// Execução que não está `ativa` não é abandonada de novo (já terminou de algum jeito): devolve
/// `conflito`, para o chamador não mascarar um desfecho com outro.
pub fn abandonar(
    estado: &Estado,
    motivo: &str,
    ator: AtorAcao,
    chave_idempotencia: Option<&str>,
) -> DomainResult<Resultado> {
    let motivo = motivo.trim();
    if motivo.is_empty() {
        return Err(DomainError::Validacao {
            campo: "motivo",
            motivo: "o abandono precisa de um motivo: é ele que fica no histórico da Ficha"
                .to_owned(),
        });
    }
    if estado.status != StatusExecucao::Ativa {
        return Err(DomainError::Conflito {
            entidade: "execução",
            id: estado.no_atual.id().to_owned(),
            motivo: format!(
                "a execução já está {} e não pode ser abandonada",
                estado.status
            ),
        });
    }
    if let Some(chave) = chave_idempotencia
        && let Some(recibo) = estado.chaves.get(chave)
    {
        return Ok(Resultado {
            novo_estado: estado.clone(),
            eventos: Vec::new(),
            efeitos: Vec::new(),
            aguardando: recibo.aguardando.clone(),
            repetida: true,
            encerra_atendimento: false,
            erro_de_regra: false,
        });
    }

    let no = estado.no_atual.id().to_owned();
    let mut novo_estado = estado.clone();
    novo_estado.status = StatusExecucao::Abandonada;
    novo_estado.motivo = Some(motivo.to_owned());
    novo_estado.atores_que_agiram = Vec::new();

    let eventos = vec![Evento {
        tipo: TipoEventoFicha::ExecucaoAbandonada,
        ator: ator_de_evento(ator),
        payload: json!({ "motivo": motivo, "no": no }),
    }];

    if let Some(chave) = chave_idempotencia {
        novo_estado.chaves.insert(
            chave.to_owned(),
            Recibo {
                no_atual: novo_estado.no_atual.clone(),
                aguardando: Vec::new(),
            },
        );
    }

    Ok(Resultado {
        novo_estado,
        eventos,
        efeitos: Vec::new(),
        aguardando: Vec::new(),
        repetida: false,
        encerra_atendimento: false,
        erro_de_regra: false,
    })
}

/// Efeitos do `ao_entrar` de uma etapa, com o texto resolvido e a chave de cada um.
///
/// É público porque a entrada na **primeira** etapa acontece em [`iniciar`], que devolve só o
/// estado: quem grava a execução nova (`poph-store`, FX-20) pede os efeitos daqui e enfileira.
///
/// A chave é a mesma que a travessia usa (`etapa:<id>/<índice>`), então enfileirar por aqui e por
/// [`executar_acao`] não duplica efeito.
pub fn efeitos_de_entrada(
    ir: &Ir,
    etapa_id: &str,
    dados: &Dados,
    contexto: &Contexto<'_>,
) -> DomainResult<Vec<EfeitoProduzido>> {
    let etapa = etapa_por_id(ir, etapa_id)?;
    Ok(efeitos_da_etapa(etapa, dados, contexto))
}

// ------------------------------------------------------------------ decisões, fim e efeitos

/// O que uma travessia produziu além do estado.
///
/// A falha não aparece aqui porque já está no estado (`status` e `motivo`): o chamador lê de um
/// lugar só. A exceção é [`Desfecho::erro_de_regra`], que o estado não distingue do resto (o motivo
/// é texto).
#[derive(Debug, Clone, Default)]
struct Desfecho {
    /// A execução chegou a um `fim(encerra_atendimento=True)`.
    encerra_atendimento: bool,
    /// A travessia parou porque uma regra não avaliou.
    erro_de_regra: bool,
}

/// Atravessa o grafo a partir de `destino` até parar numa etapa (segue) ou num fim (conclui).
///
/// Decisão é avaliada na hora, com os dados já gravados, e o ramo é seguido na mesma passagem; cada
/// uma vira um evento `decisao_resolvida`. Ciclo (a mesma decisão duas vezes) e estouro do teto de
/// saltos param a execução em `falha`, com o motivo no estado e no evento `execucao_falhou` — a
/// Ficha continua aberta, porque quem fecha é o atendimento, não o fluxo.
///
/// Nó que a estrutura não tem (`etapa` ou `decisao`) é [`DomainError::IrDivergente`]: a IR gravada
/// diverge e o motor falha fechado.
fn seguir(
    ir: &Ir,
    estado: &mut Estado,
    mut destino: Destino,
    contexto: &Contexto<'_>,
    eventos: &mut Vec<Evento>,
    efeitos: &mut Vec<EfeitoProduzido>,
) -> DomainResult<Desfecho> {
    let mut saltos = 0usize;
    let mut visitadas: Vec<String> = Vec::new();

    loop {
        match destino.clone() {
            Destino::Etapa { id } => {
                let etapa = etapa_por_id(ir, &id)?;
                estado.no_atual = Destino::Etapa { id: id.clone() };
                estado.atores_que_agiram = Vec::new();
                eventos.push(Evento {
                    tipo: TipoEventoFicha::EtapaEntrou,
                    ator: AtorEvento::Sistema,
                    payload: json!({ "etapa": id }),
                });
                let da_entrada = efeitos_da_etapa(etapa, &estado.dados, contexto);
                for efeito in da_entrada {
                    eventos.push(evento_do_efeito(&efeito));
                    efeitos.push(efeito);
                }
                return Ok(Desfecho::default());
            }
            Destino::Decisao { id } => {
                let decisao = decisao_por_id(ir, &id)?;
                if visitadas.contains(&id) {
                    let caminho = format!("{} → {id}", visitadas.join(" → "));
                    let motivo = format!(
                        "ciclo de decisões: {caminho}; a execução parou para não repetir o caminho \
                         (a publicação recusa este ciclo com FLX23)"
                    );
                    return Ok(parar_em_falha(
                        estado,
                        Destino::Decisao { id },
                        motivo,
                        eventos,
                        false,
                    ));
                }
                visitadas.push(id.clone());
                saltos += 1;
                if saltos > contexto.max_saltos_decisao {
                    let motivo = format!(
                        "a execução passou de {} decisões encadeadas (fluxos.max_saltos_decisao); \
                         a estrutura do fluxo entrou em laço",
                        contexto.max_saltos_decisao
                    );
                    return Ok(parar_em_falha(
                        estado,
                        Destino::Decisao { id },
                        motivo,
                        eventos,
                        false,
                    ));
                }
                let escolha = match contexto.avaliador.avaliar(&decisao.se, &estado.dados) {
                    Ok(escolha) => escolha,
                    // Regra defeituosa ou cara é falha da **execução** (FLXR01/FLXR03), não da
                    // chamada: a Ficha continua aberta e o atendente vê o motivo.
                    Err(erro) => {
                        return Ok(parar_em_falha(
                            estado,
                            Destino::Decisao { id },
                            erro.to_string(),
                            eventos,
                            true,
                        ));
                    }
                };
                let ramo = if escolha {
                    decisao.entao.clone()
                } else {
                    decisao.senao.clone()
                };
                eventos.push(Evento {
                    tipo: TipoEventoFicha::DecisaoResolvida,
                    ator: AtorEvento::Sistema,
                    payload: json!({ "decisao": id, "resultado": escolha, "destino": ramo }),
                });
                destino = ramo;
            }
            Destino::Fim {
                id,
                resultado,
                encerra_atendimento,
            } => {
                estado.status = StatusExecucao::Concluida;
                estado.resultado = resultado.clone();
                estado.no_atual = Destino::Fim {
                    id: id.clone(),
                    resultado: resultado.clone(),
                    encerra_atendimento,
                };
                eventos.push(Evento {
                    tipo: TipoEventoFicha::ExecucaoConcluida,
                    ator: AtorEvento::Sistema,
                    payload: json!({
                        "fim": id,
                        "resultado": resultado,
                        "encerra_atendimento": encerra_atendimento,
                    }),
                });
                return Ok(Desfecho {
                    encerra_atendimento,
                    erro_de_regra: false,
                });
            }
        }
    }
}

/// Marca a execução como `falha` no nó em que ela parou, com o motivo no estado e no evento.
///
/// `erro_de_regra` separa a regra que não avalia das outras duas causas de falha (ciclo e teto de
/// saltos), que o estado guarda só como texto.
fn parar_em_falha(
    estado: &mut Estado,
    no: Destino,
    motivo: String,
    eventos: &mut Vec<Evento>,
    erro_de_regra: bool,
) -> Desfecho {
    estado.status = StatusExecucao::Falha;
    estado.motivo = Some(motivo.clone());
    estado.atores_que_agiram = Vec::new();
    estado.no_atual = no;
    eventos.push(Evento {
        tipo: TipoEventoFicha::ExecucaoFalhou,
        ator: AtorEvento::Sistema,
        payload: json!({ "no": estado.no_atual.id(), "motivo": motivo }),
    });
    Desfecho {
        encerra_atendimento: false,
        erro_de_regra,
    }
}

/// Resultado de uma passagem que terminou sem ação: a execução concluiu ou parou em `falha`.
fn resultado_parado(
    estado: Estado,
    eventos: Vec<Evento>,
    efeitos: Vec<EfeitoProduzido>,
    erro_de_regra: bool,
) -> Resultado {
    Resultado {
        novo_estado: estado,
        eventos,
        efeitos,
        aguardando: Vec::new(),
        repetida: false,
        encerra_atendimento: false,
        erro_de_regra,
    }
}

/// Efeitos do `ao_entrar` de uma etapa (lista declarada), com a chave de cada um.
fn efeitos_da_etapa(etapa: &Etapa, dados: &Dados, contexto: &Contexto<'_>) -> Vec<EfeitoProduzido> {
    efeitos_declarados(
        &etapa.ao_entrar,
        format!("etapa:{}", etapa.id),
        dados,
        contexto,
    )
}

/// Resolve o texto de cada efeito declarado e monta a chave `nó/índice`.
///
/// Interpola o **texto** (o que o cliente e a equipe leem): `{etapa.campo}`, `{ficha.id}` e
/// `{cliente.peer}`. O `para` do `notificar_whatsapp` e o departamento do `encaminhar` são
/// identificadores e passam como estão.
fn efeitos_declarados(
    declarados: &[Efeito],
    no: String,
    dados: &Dados,
    contexto: &Contexto<'_>,
) -> Vec<EfeitoProduzido> {
    declarados
        .iter()
        .enumerate()
        .map(|(indice, declarado)| {
            let efeito = match declarado {
                Efeito::MensagemAoCliente { texto } => Efeito::MensagemAoCliente {
                    texto: interpolar(texto, dados, contexto),
                },
                Efeito::NotificarWhatsapp { para, mensagem } => Efeito::NotificarWhatsapp {
                    para: para.clone(),
                    mensagem: interpolar(mensagem, dados, contexto),
                },
                Efeito::Encaminhar { departamento } => Efeito::Encaminhar {
                    departamento: departamento.clone(),
                },
            };
            EfeitoProduzido {
                chave: format!("{no}/{indice}"),
                no: no.clone(),
                indice,
                efeito,
            }
        })
        .collect()
}

/// O evento que registra na Ficha que um efeito entrou na fila.
fn evento_do_efeito(efeito: &EfeitoProduzido) -> Evento {
    Evento {
        tipo: TipoEventoFicha::EfeitoCriado,
        ator: AtorEvento::Sistema,
        payload: json!({ "chave": efeito.chave, "tipo": efeito.tipo(), "no": efeito.no }),
    }
}

/// Troca as variáveis do texto por valor: `{etapa.campo}`, `{ficha.id}`, `{cliente.peer}`.
///
/// Variável que não existe (ou `{` sem par) vira texto vazio / fica como está: o efeito sai sempre,
/// porque a publicação já avisou o autor com `FLX59`. Isto é **texto de efeito**, não regra
/// avaliada: nada aqui executa o que o cliente escreveu.
fn interpolar(texto: &str, dados: &Dados, contexto: &Contexto<'_>) -> String {
    let mut saida = String::with_capacity(texto.len());
    let mut resto = texto;
    while let Some(inicio) = resto.find('{') {
        saida.push_str(&resto[..inicio]);
        let depois = &resto[inicio + 1..];
        match depois.find('}') {
            Some(fim) => {
                saida.push_str(&valor_da_variavel(&depois[..fim], dados, contexto));
                resto = &depois[fim + 1..];
            }
            None => {
                // `{` sem fechamento: não é variável, é texto do autor.
                saida.push_str(&resto[inicio..]);
                resto = "";
            }
        }
    }
    saida.push_str(resto);
    saida
}

/// Valor de uma variável de efeito, em texto canônico (§5); vazio quando ela não existe.
fn valor_da_variavel(nome: &str, dados: &Dados, contexto: &Contexto<'_>) -> String {
    match nome.trim() {
        "ficha.id" => contexto.ficha_id.to_owned(),
        "cliente.peer" => contexto.cliente_peer.to_owned(),
        nome => match nome.split_once('.') {
            Some((etapa, campo)) => dados
                .get(etapa)
                .and_then(|valores| valores.get(campo))
                .and_then(texto_canonico)
                .unwrap_or_default(),
            None => String::new(),
        },
    }
}

/// Decisão da IR pelo id; IR que aponta para decisão inexistente é divergência, não pânico.
fn decisao_por_id<'a>(ir: &'a Ir, id: &str) -> DomainResult<&'a Decisao> {
    ir.decisoes
        .iter()
        .find(|decisao| decisao.id == id)
        .ok_or_else(|| DomainError::IrDivergente {
            fluxo: ir.fluxo.id.clone(),
            motivo: format!("a decisão `{id}` não existe na estrutura do fluxo"),
        })
}

/// Atores que precisam agir para a etapa avançar, na ordem alfabética do texto.
///
/// Deriva das **ações** (nunca de `atores_acao`): só a ação que leva para frente (etapa ou decisão)
/// e tem ator restrito cria exigência. Ação irrestrita, ou que vai para `fim(...)`, não exige
/// ninguém — é o mesmo cálculo de `state_engine.required_actor_types`.
pub fn atores_exigidos(etapa: &Etapa) -> Vec<AtorAcao> {
    let mut exigidos: Vec<AtorAcao> = Vec::new();
    for acao in &etapa.acoes {
        if matches!(acao.vai_para, Destino::Fim { .. }) {
            continue;
        }
        let Some(ator) = acao.ator else {
            continue;
        };
        if !exigidos.contains(&ator) {
            exigidos.push(ator);
        }
    }
    ordena(&mut exigidos);
    exigidos
}

/// Campos **obrigatórios** da etapa que ainda não foram preenchidos, na ordem do script.
pub fn campos_faltando(etapa: &Etapa, dados: &Dados) -> Vec<String> {
    let registrados = dados_da_etapa(dados, &etapa.id);
    etapa
        .campos
        .iter()
        .filter(|campo| campo.obrigatorio && !preenchido(registrados.get(&campo.id)))
        .map(|campo| campo.id.clone())
        .collect()
}

/// Os campos já registrados de uma etapa (`dados["etapa"]`), vazios quando não há nada.
///
/// Valor que não é objeto é tratado como etapa sem campo: o motor não conserta dado que não é dele.
pub fn dados_da_etapa(dados: &Dados, etapa: &str) -> Campos {
    match dados.get(etapa) {
        Some(Json::Object(mapa)) => mapa.clone(),
        _ => Campos::new(),
    }
}

// ------------------------------------------------------------------ auxiliares internos

/// Etapa da IR pelo id; IR que aponta para etapa inexistente é divergência, não pânico.
fn etapa_por_id<'a>(ir: &'a Ir, id: &str) -> DomainResult<&'a Etapa> {
    ir.etapas
        .iter()
        .find(|etapa| etapa.id == id)
        .ok_or_else(|| DomainError::IrDivergente {
            fluxo: ir.fluxo.id.clone(),
            motivo: format!("a etapa `{id}` não existe na estrutura do fluxo"),
        })
}

/// Recusa de ação (`acao_invalida`): sempre com o nome da ação e o motivo que o agente lê.
fn recusa(acao: &str, motivo: String) -> DomainError {
    DomainError::AcaoInvalida {
        acao: acao.to_owned(),
        motivo,
    }
}

/// Recusa de campos (`campos_invalidos`), com a etapa e a lista de problemas.
fn campos_invalidos(etapa: &Etapa, erros: Vec<String>) -> DomainError {
    DomainError::CamposDaEtapaInvalidos {
        etapa: etapa.id.clone(),
        erros,
    }
}

/// Ordem estável dos atores: o texto (`agente` antes de `atendente`), igual ao `sorted()` do Python.
fn ordena(atores: &mut [AtorAcao]) {
    atores.sort_by_key(|ator| ator.como_texto());
}

fn ator_de_evento(ator: AtorAcao) -> AtorEvento {
    match ator {
        AtorAcao::Agente => AtorEvento::Agente,
        AtorAcao::Atendente => AtorEvento::Atendente,
    }
}

/// Confere cada campo informado e devolve os valores a gravar (texto sem espaços nas pontas).
///
/// Os erros saem em ordem alfabética do campo. Ordenar de propósito (e não confiar na ordem do
/// mapa) é o que mantém a mensagem igual nos dois builds: no workspace o `serde_json` compila com
/// `preserve_order` (por causa do `utoipa` do `poph-server`) e o mapa passa a guardar a ordem de
/// inserção; sozinho, ele ordena por chave. A resposta ao agente não pode depender disso.
fn validar_campos(etapa: &Etapa, campos: &Campos) -> Result<Campos, Vec<String>> {
    let mut validados = Campos::new();
    let mut erros: Vec<String> = Vec::new();
    for (chave, valor) in campos {
        let Some(campo) = etapa.campos.iter().find(|candidato| candidato.id == *chave) else {
            erros.push(campo_de_fora_da_etapa(etapa, chave));
            continue;
        };
        match validar_valor(campo, valor) {
            Ok(normalizado) => {
                validados.insert(chave.clone(), normalizado);
            }
            Err(motivo) => erros.push(format!("{chave}: {motivo}")),
        }
    }
    if erros.is_empty() {
        Ok(validados)
    } else {
        erros.sort();
        Err(erros)
    }
}

/// O valor de um campo contra o tipo declarado; devolve o valor a gravar.
///
/// `null` é "não preenchido": aceito no campo opcional, recusado no obrigatório.
fn validar_valor(campo: &Campo, valor: &Json) -> Result<Json, String> {
    if valor.is_null() {
        return if campo.obrigatorio {
            Err("é obrigatório e não aceita valor vazio".to_owned())
        } else {
            Ok(Json::Null)
        };
    }
    match campo.tipo {
        TipoCampo::Texto => match valor {
            Json::String(texto) => {
                let limpo = texto.trim();
                if campo.obrigatorio && limpo.is_empty() {
                    return Err("é obrigatório e veio vazio".to_owned());
                }
                Ok(Json::String(limpo.to_owned()))
            }
            outro => Err(esperava("texto", outro)),
        },
        TipoCampo::Numero => match valor {
            Json::Number(_) => Ok(valor.clone()),
            outro => Err(esperava("número", outro)),
        },
        TipoCampo::Data => match valor {
            Json::String(texto) if data_iso_valida(texto.trim()) => {
                Ok(Json::String(texto.trim().to_owned()))
            }
            Json::String(texto) => Err(format!(
                "esperava data ISO 8601 (`AAAA-MM-DD` ou `AAAA-MM-DDTHH:MM:SS-03:00`) e veio `{texto}`"
            )),
            outro => Err(esperava("data em texto ISO 8601", outro)),
        },
        TipoCampo::Booleano => match valor {
            Json::Bool(_) => Ok(valor.clone()),
            outro => Err(esperava("booleano (`true`/`false`)", outro)),
        },
        TipoCampo::Opcao => match valor {
            Json::String(texto) => {
                let limpo = texto.trim();
                if campo.opcoes.iter().any(|opcao| opcao == limpo) {
                    Ok(Json::String(limpo.to_owned()))
                } else {
                    Err(format!(
                        "esperava uma das opções: {}",
                        lista_de_opcoes(campo)
                    ))
                }
            }
            outro => Err(esperava("uma das opções em texto", outro)),
        },
        TipoCampo::Anexo => match valor {
            Json::Object(mapa) if matches!(mapa.get("nome"), Some(Json::String(nome)) if !nome.trim().is_empty()) => {
                Ok(valor.clone())
            }
            Json::Object(_) => Err("esperava um anexo com o nome do arquivo em `nome`".to_owned()),
            outro => Err(esperava("anexo (`{nome, tipo, tamanho}`)", outro)),
        },
    }
}

fn esperava(esperado: &str, valor: &Json) -> String {
    format!("esperava {esperado} e veio {}", tipo_do_json(valor))
}

fn tipo_do_json(valor: &Json) -> &'static str {
    match valor {
        Json::Null => "nulo",
        Json::Bool(_) => "booleano",
        Json::Number(_) => "número",
        Json::String(_) => "texto",
        Json::Array(_) => "lista",
        Json::Object(_) => "objeto",
    }
}

fn campo_de_fora_da_etapa(etapa: &Etapa, chave: &str) -> String {
    let mut mensagem = format!(
        "`{chave}` não é campo da etapa `{}`; os campos desta etapa são: {}",
        etapa.id,
        lista_de_campos(etapa)
    );
    if chave.contains('.') {
        mensagem.push_str(" (informe só o nome do campo, sem o prefixo da etapa)");
    }
    mensagem
}

fn lista_de_acoes(etapa: &Etapa) -> String {
    if etapa.acoes.is_empty() {
        return "nenhuma (a etapa não tem saída)".to_owned();
    }
    etapa
        .acoes
        .iter()
        .map(|acao| format!("`{}`", acao.id))
        .collect::<Vec<_>>()
        .join(", ")
}

fn lista_de_campos(etapa: &Etapa) -> String {
    if etapa.campos.is_empty() {
        return "nenhum".to_owned();
    }
    etapa
        .campos
        .iter()
        .map(|campo| format!("`{}`", campo.id))
        .collect::<Vec<_>>()
        .join(", ")
}

fn lista_de_opcoes(campo: &Campo) -> String {
    campo
        .opcoes
        .iter()
        .map(|opcao| format!("`{opcao}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn lista_de_atores(atores: &[AtorAcao]) -> String {
    if atores.is_empty() {
        return "nenhum".to_owned();
    }
    atores
        .iter()
        .map(|ator| format!("`{ator}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Data em ISO 8601: `AAAA-MM-DD`, com hora e fuso opcionais (`AAAA-MM-DDTHH:MM`,
/// `…:SS`, `…Z`, `…-03:00`).
///
/// Confere a **forma e as faixas** (mês 1–12, dia 1–31, hora 0–23, minuto e segundo 0–59); não
/// confere o calendário (`2026-02-30` passa). Isso é de quem consome a data, não do fluxo.
fn data_iso_valida(texto: &str) -> bool {
    if !texto.is_ascii() || texto.len() < 10 {
        return false;
    }
    let (data, resto) = texto.split_at(10);
    if !data_valida(data) {
        return false;
    }
    match resto.strip_prefix('T') {
        None => resto.is_empty(),
        Some(hora) => hora_valida(hora),
    }
}

fn data_valida(data: &str) -> bool {
    let partes: Vec<&str> = data.split('-').collect();
    let [ano, mes, dia] = partes.as_slice() else {
        return false;
    };
    if ano.len() != 4 || mes.len() != 2 || dia.len() != 2 {
        return false;
    }
    if !digitos(ano) || !digitos(mes) || !digitos(dia) {
        return false;
    }
    let mes = mes.parse::<u32>().unwrap_or(0);
    let dia = dia.parse::<u32>().unwrap_or(0);
    (1..=12).contains(&mes) && (1..=31).contains(&dia)
}

fn hora_valida(hora: &str) -> bool {
    let (relogio, fuso) = match hora.find(['Z', '+', '-']) {
        Some(posicao) => hora.split_at(posicao),
        None => (hora, ""),
    };
    let (hms, fracao) = match relogio.split_once('.') {
        Some((hms, fracao)) => (hms, Some(fracao)),
        None => (relogio, None),
    };
    let partes: Vec<&str> = hms.split(':').collect();
    let (h, m, s) = match partes.as_slice() {
        [h, m] => (*h, *m, None),
        [h, m, s] => (*h, *m, Some(*s)),
        _ => return false,
    };
    let dentro = |texto: &str, teto: u32| {
        texto.len() == 2 && digitos(texto) && texto.parse::<u32>().is_ok_and(|valor| valor <= teto)
    };
    if !dentro(h, 23) || !dentro(m, 59) {
        return false;
    }
    if let Some(segundo) = s
        && !dentro(segundo, 59)
    {
        return false;
    }
    if let Some(fracao) = fracao
        && !digitos(fracao)
    {
        return false;
    }
    match fuso {
        "" | "Z" => true,
        texto if texto.starts_with('+') || texto.starts_with('-') => {
            let partes: Vec<&str> = texto[1..].split(':').collect();
            let [fuso_hora, fuso_minuto] = partes.as_slice() else {
                return false;
            };
            dentro(fuso_hora, 23) && dentro(fuso_minuto, 59)
        }
        _ => false,
    }
}

fn digitos(texto: &str) -> bool {
    !texto.is_empty() && texto.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_iso_aceita_as_formas_do_documento() {
        for texto in [
            "2026-01-01",
            "2026-11-22T14:42",
            "2026-11-22T14:42:09",
            "2026-11-22T14:42:09Z",
            "2026-11-22T14:42:09-03:00",
            "2026-11-22T14:42:09.123+00:00",
        ] {
            assert!(data_iso_valida(texto), "recusou `{texto}`");
        }
    }

    #[test]
    fn data_iso_recusa_o_que_nao_e_iso() {
        for texto in [
            "22/11/2026",
            "2026-11",
            "2026-13-01",
            "2026-00-10",
            "2026-01-32",
            "2026-01-01T24:00",
            "2026-01-01T10:61",
            "2026-01-01T10:00:61",
            "2026-01-01 10:00",
            "2026-01-01T10:00+3:00",
            "2026-01-01x",
            "ontem",
            "",
        ] {
            assert!(!data_iso_valida(texto), "aceitou `{texto}`");
        }
    }

    #[test]
    fn tipo_do_json_descreve_o_que_veio() {
        // A mensagem de recusa cita o que chegou: é o que o agente lê para corrigir a chamada.
        assert_eq!(tipo_do_json(&json!("x")), "texto");
        assert_eq!(tipo_do_json(&json!(1)), "número");
        assert_eq!(tipo_do_json(&json!(true)), "booleano");
        assert_eq!(tipo_do_json(&json!([])), "lista");
        assert_eq!(tipo_do_json(&json!({})), "objeto");
        assert_eq!(tipo_do_json(&Json::Null), "nulo");
    }

    #[test]
    fn a_ordem_dos_erros_de_campo_e_alfabetica_e_nao_a_do_mapa() {
        // No build do workspace o `serde_json` compila com `preserve_order` (o `utoipa` do
        // `poph-server` pede) e o mapa guarda a ordem de inserção; sozinho, ele ordena por chave.
        // A mensagem que o agente lê não pode depender de qual dos dois builds rodou.
        let etapa = Etapa {
            id: "cadastro".to_owned(),
            nome: "Cadastro".to_owned(),
            instrucao: None,
            atores_acao: vec![AtorAcao::Agente],
            campos: Vec::new(),
            acoes: Vec::new(),
            ao_entrar: Vec::new(),
        };
        let mut campos = Campos::new();
        for chave in ["zeta", "alfa", "meio"] {
            campos.insert(chave.to_owned(), json!("x"));
        }

        let erros = validar_campos(&etapa, &campos).expect_err("a etapa não tem campo nenhum");
        assert_eq!(erros.len(), 3);
        assert!(erros[0].starts_with("`alfa`"), "{erros:?}");
        assert!(erros[1].starts_with("`meio`"), "{erros:?}");
        assert!(erros[2].starts_with("`zeta`"), "{erros:?}");
    }

    #[test]
    fn dados_da_etapa_ignora_o_que_nao_e_objeto() {
        let mut dados = Dados::new();
        dados.insert("coleta".to_owned(), json!({ "cep": "20000-000" }));
        dados.insert("quebrada".to_owned(), json!("não é mapa"));
        assert_eq!(
            dados_da_etapa(&dados, "coleta").get("cep"),
            Some(&json!("20000-000"))
        );
        assert!(dados_da_etapa(&dados, "quebrada").is_empty());
        assert!(dados_da_etapa(&dados, "ausente").is_empty());
    }
}
