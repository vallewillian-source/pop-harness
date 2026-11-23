//! Motor de execução de fluxos: iniciar a execução, registrar campos e executar uma ação (FX-13).
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
//!   campos e o critério `auto`; grava os campos; avança a execução quando a etapa terminou) e as
//!   consultas que o prompt e o banco precisam ([`atores_exigidos`], [`campos_faltando`],
//!   [`dados_da_etapa`]).
//! - **Não faz (FX-14):** resolver decisões, concluir a execução num `fim(...)`, abandonar e
//!   produzir os efeitos. Por isso [`Resultado::efeitos`] sai sempre vazio nesta tarefa — quem o
//!   preenche é a FX-14, com o texto já interpolado e a chave de idempotência — e uma ação que cai
//!   em decisão ou em fim deixa o estado **no nó de destino**, ainda `ativa`, para a FX-14 seguir.
//! - **Não faz (FX-20):** transação, `version`, isolamento entre Ficha e fluxo, escrita do evento.
//!   O motor descreve; o store grava.
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
//! - Decisão e fim **não** são resolvidos aqui: `no_atual` fica apontando para o nó e a FX-14 segue.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json, json};

use poph_types::{
    AtorAcao, AtorEvento, DomainError, DomainResult, StatusExecucao, TipoCampo, TipoEventoFicha,
};

use crate::ir::{Campo, Destino, Efeito, Etapa, Ir};
use crate::regras::{Dados, preenchido};

/// Valores de campo informados junto de uma ação: `campo -> valor`.
///
/// As chaves são ids de campo **da etapa atual** (`campos[].id`), sem o prefixo da etapa. O que não
/// for campo da etapa é recusado com a lista dos campos dela.
pub type Campos = Map<String, Json>;

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
    /// Resumo do desfecho (`fins[].resultado`), preenchido quando a execução chega ao fim (FX-14).
    pub resultado: Option<String>,
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

/// Desfecho de [`executar_acao`]: o estado novo, o que gravar e o que dizer ao agente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resultado {
    /// Estado da execução depois da passagem (inalterado quando `repetida`).
    pub novo_estado: Estado,
    /// Eventos a gravar, na ordem em que aconteceram: campos, ação, etapa concluída, etapa entrou.
    pub eventos: Vec<Evento>,
    /// Efeitos a enfileirar: **vazio nesta tarefa**; a FX-14 preenche com o texto resolvido.
    pub efeitos: Vec<Efeito>,
    /// Atores que ainda precisam agir porque a etapa ficou `parcial`; vazio quando avançou.
    pub aguardando: Vec<AtorAcao>,
    /// `true` quando a `chave_idempotencia` já tinha sido aplicada: nada mudou e nada é gravado.
    pub repetida: bool,
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
        chaves: BTreeMap::new(),
    })
}

/// Executa uma ação da etapa atual: confere ator, campos e o critério `auto`, grava os campos e
/// avança quando a etapa terminou.
///
/// A ordem das conferências é a do POP Atende (`state_engine.execute_action`), com a mensagem que o
/// agente precisa para se corrigir:
///
/// 1. execução `ativa` e parada numa etapa;
/// 2. chave de idempotência já aplicada → devolve o mesmo desfecho, sem evento;
/// 3. a ação existe na etapa (senão erro com a **lista das válidas**);
/// 4. o ator age nesta etapa (`atores_acao`);
/// 5. a ação não é restrita a outro ator;
/// 6. cada campo informado existe na etapa e tem o tipo/opção certo;
/// 7. todo campo de `exige` está preenchido;
/// 8. então grava os campos, decide o avanço e monta os eventos.
///
/// Nada disso grava pela metade: qualquer recusa deixa o estado como estava (o chamador fica com o
/// `estado` que passou).
pub fn executar_acao(
    ir: &Ir,
    estado: &Estado,
    acao: &str,
    campos: &Campos,
    ator: AtorAcao,
    chave_idempotencia: Option<&str>,
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
        });
    }

    // 3. A execução precisa estar numa etapa: decisão e fim são da FX-14.
    let etapa_id = match &estado.no_atual {
        Destino::Etapa { id } => id.clone(),
        Destino::Decisao { id } => {
            return Err(recusa(
                acao,
                format!(
                    "a execução está na decisão `{id}`: ela precisa ser resolvida antes de qualquer ação \
                     (o motor não decide sozinho)"
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

    // 4. A ação existe nesta etapa (sem busca em largura: só as ações daqui).
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

    // 5. O ator age nesta etapa.
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

    // 6. A ação não é de outro ator.
    if let Some(dono) = acao_ir.ator
        && dono != ator
    {
        return Err(recusa(
            acao,
            format!("a ação `{acao}` é restrita ao ator `{dono}`"),
        ));
    }

    // 7. Campos: existem na etapa, tipo certo, opção da lista. Nada disso grava pela metade.
    let validados = match validar_campos(etapa, campos) {
        Ok(validados) => validados,
        Err(erros) => return Err(campos_invalidos(etapa, erros)),
    };

    // 8. `exige`: os campos da própria etapa, contando o que já estava gravado.
    let mut registrados = dados_da_etapa(&estado.dados, &etapa.id);
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

    // 9. Critério `auto`: quem falta agir para a etapa avançar.
    let mut progresso = estado.atores_que_agiram.clone();
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

    // 10. Destino existente: uma IR gravada à mão não pode deixar a execução apontando para uma
    // etapa que não existe (a publicação recusa isto em FX-11; aqui a falha é fechada).
    if avanca && let Destino::Etapa { id } = &acao_ir.vai_para {
        etapa_por_id(ir, id)?;
    }

    // 11. Eventos e estado novo.
    let autor = ator_de_evento(ator);
    let mut eventos = Vec::new();
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

    let mut novo_estado = estado.clone();
    novo_estado
        .dados
        .insert(etapa.id.clone(), Json::Object(registrados));

    let aguardando_do_resultado = if avanca {
        eventos.push(Evento {
            tipo: TipoEventoFicha::EtapaConcluida,
            ator: AtorEvento::Sistema,
            payload: json!({ "etapa": etapa.id, "acao": acao }),
        });
        if let Destino::Etapa { id } = &acao_ir.vai_para {
            eventos.push(Evento {
                tipo: TipoEventoFicha::EtapaEntrou,
                ator: AtorEvento::Sistema,
                payload: json!({ "etapa": id }),
            });
        }
        novo_estado.no_atual = acao_ir.vai_para.clone();
        novo_estado.atores_que_agiram = Vec::new();
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
        efeitos: Vec::new(),
        aguardando: aguardando_do_resultado,
        repetida: false,
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
