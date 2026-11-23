//! As ferramentas do fluxo no turno (FX-24, PRD §6.5).
//!
//! Três ferramentas, registradas no mesmo [`RegistroPadrao`] das outras e filtradas pela lista
//! habilitada do agente (D-04):
//!
//! | Ferramenta | Quando é oferecida | O que faz |
//! |---|---|---|
//! | `fluxo_executar_acao(acao, campos?, chave_idempotencia?)` | com execução ativa na Ficha | valida e grava a ação da etapa atual, ator `agente` |
//! | `fluxo_abandonar(motivo)` | com execução ativa na Ficha | abandona a execução, ator `agente` |
//! | `fluxo_iniciar(fluxo_id)` | **só na reserva** do Jev (§6.3-4) | valida como o G9 e abre a execução, ator `agente` |
//!
//! Regras que este módulo respeita:
//!
//! - **O ator é do código.** O modelo não escolhe quem age: a ferramenta sempre usa
//!   [`AtorAcao::Agente`]. Nada de `actor_type` vindo dos argumentos.
//! - **Sem auto-transição.** A ação que não existe na etapa atual é recusa com a lista das válidas,
//!   e nada é gravado; o motor não executa ações intermediárias por conta própria.
//! - **A transição é a da FX-20.** O motor devolve o desfecho (puro) e a gravação é
//!   [`poph_store::Store::execucoes`] (`aplicar_transicao`), numa escrita só, com eventos, efeitos e
//!   fechamento do atendimento na mesma transação. A tradução é a de [`crate::fluxos_transicao`].
//! - **Efeitos saem depois do texto final.** A ferramenta só enfileira (`fluxo_efeitos`); quem
//!   entrega é o laço do servidor, que espera o turno da conversa terminar (FX-21).
//! - **A resposta ao modelo não tem id interno.** Só o que ele precisa para responder e agir: o
//!   nome do fluxo, a etapa, os campos que faltam, quem falta agir e as ações válidas.
//!
//! A reserva é o único caminho que oferece `fluxo_iniciar`: com o Jev respondendo (`nenhum` ou uma
//! escolha confiável), quem abre a execução é o código, antes do modelo (FX-23). A ferramenta existe
//! para quando o Jev falha ou o gate está desligado (§6.3-4).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use poph_fluxos::ir::{Destino, Ir};
use poph_fluxos::motor::{
    self, Contexto as ContextoDoMotor, Estado as EstadoDoMotor, executar_acao,
};
use poph_store::{ErroStore, Execucao, ExtrasDaTransicao, Ficha, Store};
use poph_types::{AtorAcao, FluxosConfig, StatusFicha};
use serde_json::{Map, Value};

use super::ferramentas::{ContextoFerramenta, FerramentaInterna, FuturoFerramenta, RegistroPadrao};
use super::fluxo::{self, AjustesDoFluxo, QuemInicia};
use crate::fluxos_transicao::transicao_do_motor;

/// Nome da ferramenta que executa uma ação da etapa atual.
pub const NOME_EXECUTAR_ACAO: &str = "fluxo_executar_acao";
/// Nome da ferramenta que abandona a execução em andamento.
pub const NOME_ABANDONAR: &str = "fluxo_abandonar";
/// Nome da ferramenta da reserva (repetido aqui para o turno filtrar sem importar dois módulos).
pub const NOME_INICIAR: &str = fluxo::NOME_FERRAMENTA_INICIAR;

/// `true` quando o nome é de uma ferramenta do fluxo (o turno usa para oferecer só no momento certo).
pub fn eh_ferramenta_do_fluxo(nome: &str) -> bool {
    matches!(nome, NOME_EXECUTAR_ACAO | NOME_ABANDONAR | NOME_INICIAR)
}

/// O que as três ferramentas compartilham: o banco e os ajustes de `[fluxos]`.
#[derive(Clone)]
struct FluxoDoTurno {
    store: Arc<Store>,
    ajustes: AjustesDoFluxo,
}

/// Registra as ferramentas do fluxo no registro do turno.
///
/// Motor de fluxos desligado (`[fluxos] ativo = false`) não registra nada: o PRD diz que a chave
/// desliga "o motor (G9, prompt, ferramentas, efeitos)" e a Ficha continua funcionando.
pub fn registrar(
    registro: RegistroPadrao,
    store: Arc<Store>,
    config: &FluxosConfig,
) -> RegistroPadrao {
    registrar_com(registro, store, AjustesDoFluxo::da_config(config))
}

/// Igual a [`registrar`], com os ajustes prontos — é o que permite compartilhar o cache de revisões
/// de quem já tem um (a API de fluxos tem o dela).
pub fn registrar_com(
    registro: RegistroPadrao,
    store: Arc<Store>,
    ajustes: AjustesDoFluxo,
) -> RegistroPadrao {
    if !ajustes.ativo() {
        return registro;
    }
    let comum = FluxoDoTurno { store, ajustes };
    registro
        .com(Arc::new(ExecutarAcao(comum.clone())) as Arc<dyn FerramentaInterna>)
        .com(Arc::new(Abandonar(comum.clone())) as Arc<dyn FerramentaInterna>)
        .com(Arc::new(Iniciar(comum)) as Arc<dyn FerramentaInterna>)
}

/// Registra o encerramento independentemente da configuração do motor de fluxos.
pub fn registrar_encerramento(registro: RegistroPadrao, store: Arc<Store>) -> RegistroPadrao {
    registro.com(Arc::new(EncerrarAtendimento(store)) as Arc<dyn FerramentaInterna>)
}

struct EncerrarAtendimento(Arc<Store>);

impl FerramentaInterna for EncerrarAtendimento {
    fn nome(&self) -> &'static str {
        "atendimento_encerrar"
    }
    fn descricao(&self) -> &'static str {
        "Encerra o atendimento quando concluído. Escreva a despedida na resposta; esta ferramenta não envia mensagem."
    }
    fn parametros(&self) -> Value {
        serde_json::json!({"type":"object", "properties":{"motivo":{"type":"string"}},
            "required":["motivo"], "additionalProperties":false})
    }
    fn executar(&self, contexto: ContextoFerramenta, argumentos: Value) -> FuturoFerramenta {
        let store = Arc::clone(&self.0);
        Box::pin(async move {
            let motivo = texto_obrigatorio(&argumentos, "motivo")?;
            match store.conversas().fechar_pelo_agente(
                contexto.org,
                contexto.conversa,
                contexto.agente,
                motivo,
            ) {
                Ok(true) => {
                    Ok("Atendimento encerrado. Responda ao cliente com a despedida.".to_owned())
                }
                Ok(false) => Ok("Atendimento já encerrado; nada mudou.".to_owned()),
                Err(_) => Err("Não foi possível encerrar o atendimento.".to_owned()),
            }
        })
    }
}

// ────────────────────────────────────────────────────────────── fluxo_executar_acao

/// `fluxo_executar_acao`: o agente executa uma ação da etapa em que a execução está.
struct ExecutarAcao(FluxoDoTurno);

impl FerramentaInterna for ExecutarAcao {
    fn nome(&self) -> &'static str {
        NOME_EXECUTAR_ACAO
    }

    fn descricao(&self) -> &'static str {
        "Executa uma ação da etapa atual do fluxo de atendimento em andamento (a lista de ações \
         válidas está no bloco <fluxo_em_andamento> do prompt). Use com os campos que a etapa \
         coleta quando o cliente já os informou. Se a ação não existir na etapa, a resposta traz as \
         válidas e nada muda."
    }

    fn parametros(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "acao": {
                    "type": "string",
                    "description": "Id da ação, exatamente como aparece em `Ações válidas` no bloco do fluxo."
                },
                "campos": {
                    "type": "object",
                    "description": "Campos da etapa que o cliente informou nesta mensagem, no formato {\"campo\": \"valor\"}."
                },
                "chave_idempotencia": {
                    "type": "string",
                    "description": "Opcional. Repetir a mesma chave devolve o mesmo desfecho, sem gravar de novo. Sem ela, a chave é derivada da própria ação."
                }
            },
            "required": ["acao"],
            "additionalProperties": false
        })
    }

    fn executar(&self, contexto: ContextoFerramenta, argumentos: Value) -> FuturoFerramenta {
        let comum = self.0.clone();
        Box::pin(async move { comum.executar_acao(contexto, argumentos) })
    }
}

impl FluxoDoTurno {
    /// Valida e grava uma ação do agente na execução ativa da Ficha da conversa.
    fn executar_acao(
        &self,
        contexto: ContextoFerramenta,
        argumentos: Value,
    ) -> Result<String, String> {
        let acao = texto_obrigatorio(&argumentos, "acao")?;
        let campos = campos_do_argumento(&argumentos)?;
        let informada = chave_opcional(&argumentos)?;
        let (ficha, execucao) = self.ficha_e_execucao(&contexto)?;
        // A chave derivada fecha o caso de o modelo repetir a mesma chamada na mesma etapa sem
        // informar chave: o banco devolve o desfecho anterior em vez de gravar outra vez.
        let chave = informada.unwrap_or_else(|| {
            chave_derivada(
                &execucao,
                &execucao.no_atual,
                &acao,
                &Value::Object(campos.clone()),
            )
        });
        if let Some(estado) = self.repeticao_ja_aplicada(&contexto, &execucao, &chave)? {
            return Ok(format!(
                "Esta ação já tinha sido registrada; nada mudou.\n{}",
                estado
            ));
        }
        let carga =
            match fluxo::carga_para_agir(&self.ajustes, &self.store, contexto.org, &execucao) {
                Some(carga) => carga,
                None => return Err(nao_consegui_ler_o_fluxo()),
            };
        let texto_da_ficha = ficha.id.to_string();
        let mut motor_contexto = ContextoDoMotor::novo(&carga.avaliador)
            .com_ficha(&texto_da_ficha)
            .com_limite_de_saltos(self.ajustes.saltos());
        if let Some(peer) = ficha.peer_externo.as_deref() {
            motor_contexto = motor_contexto.com_cliente(peer);
        }
        let resultado = executar_acao(
            &carga.ir,
            &carga.estado,
            &acao,
            &campos,
            AtorAcao::Agente,
            Some(&chave),
            &motor_contexto,
        )
        .map_err(|erro| {
            // A recusa do motor já diz o motivo e as ações válidas; a descrição do estado
            // acrescenta o que ele nem sempre repete (campos que faltam, quem falta agir).
            format!("erro: {erro}\n{}", descrever(&carga.ir, &carga.estado))
        })?;
        let gravada = self
            .store
            .execucoes()
            .aplicar_transicao(
                contexto.org,
                execucao.id,
                execucao.versao,
                &transicao_do_motor(&resultado),
                &chave,
                ExtrasDaTransicao {
                    // Sem token: o agente não tem identidade de atendente. O evento já é assinado
                    // com o ator `agente` pelo motor.
                    ator_ref: None,
                    fechada_por: None,
                    motivo_encerramento: None,
                },
            )
            .map_err(|erro| erro_da_transicao(erro, &acao))?;
        Ok(resposta_da_acao(
            &carga.ir,
            &carga.estado,
            &resultado,
            gravada.repetida,
        ))
    }

    /// Estado atual em texto quando a chave já foi aplicada; `None` quando é a primeira vez.
    fn repeticao_ja_aplicada(
        &self,
        contexto: &ContextoFerramenta,
        execucao: &Execucao,
        chave: &str,
    ) -> Result<Option<String>, String> {
        let aplicada = self
            .store
            .execucoes()
            .transicao_ja_aplicada(contexto.org, execucao.id, chave)
            .map_err(|erro| falha_de_leitura(erro, "a chave da ação"))?;
        if !aplicada {
            return Ok(None);
        }
        let Some(carga) =
            fluxo::carga_para_agir(&self.ajustes, &self.store, contexto.org, execucao)
        else {
            return Ok(Some(String::new()));
        };
        Ok(Some(descrever(&carga.ir, &carga.estado)))
    }

    /// A Ficha da conversa e a execução ativa dela, com as recusas que o modelo precisa entender.
    fn ficha_e_execucao(&self, contexto: &ContextoFerramenta) -> Result<(Ficha, Execucao), String> {
        let ficha = self
            .store
            .fichas()
            .por_conversa(contexto.org, contexto.conversa)
            .map_err(|erro| falha_de_leitura(erro, "a Ficha do atendimento"))?
            .ok_or_else(|| {
                "erro: este atendimento ainda não tem Ficha; não há fluxo para agir".to_owned()
            })?;
        if ficha.status == StatusFicha::Fechada {
            return Err(
                "erro: o atendimento já foi encerrado; a próxima mensagem do cliente abre outro"
                    .to_owned(),
            );
        }
        let execucao = self
            .store
            .execucoes()
            .ativa_da_ficha(contexto.org, ficha.id)
            .map_err(|erro| falha_de_leitura(erro, "a execução do fluxo"))?
            .ok_or_else(|| {
                "erro: não há fluxo em andamento neste atendimento; siga a conversa normalmente"
                    .to_owned()
            })?;
        Ok((ficha, execucao))
    }
}

// ───────────────────────────────────────────────────────────────── fluxo_abandonar

/// `fluxo_abandonar`: o agente para a execução onde ela está (não gera efeito).
struct Abandonar(FluxoDoTurno);

impl FerramentaInterna for Abandonar {
    fn nome(&self) -> &'static str {
        NOME_ABANDONAR
    }

    fn descricao(&self) -> &'static str {
        "Abandona o fluxo de atendimento em andamento, com um motivo curto que fica no histórico. \
         Use quando o cliente desistir ou mudar de assunto e o roteiro não servir mais. Não envia \
         nada ao cliente: a despedida é o texto da sua resposta."
    }

    fn parametros(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "motivo": {
                    "type": "string",
                    "description": "Por que o fluxo está sendo abandonado, em poucas palavras."
                },
                "chave_idempotencia": {
                    "type": "string",
                    "description": "Opcional. Repetir a mesma chave não repete o evento de abandono."
                }
            },
            "required": ["motivo"],
            "additionalProperties": false
        })
    }

    fn executar(&self, contexto: ContextoFerramenta, argumentos: Value) -> FuturoFerramenta {
        let comum = self.0.clone();
        Box::pin(async move { comum.executar_abandono(contexto, argumentos) })
    }
}

impl FluxoDoTurno {
    /// Abandona a execução ativa da Ficha da conversa.
    fn executar_abandono(
        &self,
        contexto: ContextoFerramenta,
        argumentos: Value,
    ) -> Result<String, String> {
        let motivo = texto_obrigatorio(&argumentos, "motivo")?;
        let informada = chave_opcional(&argumentos)?;
        let (_ficha, execucao) = self.ficha_e_execucao(&contexto)?;
        let chave = informada.unwrap_or_else(|| {
            chave_derivada(&execucao, &execucao.no_atual, "abandonar", &Value::Null)
        });
        if self
            .store
            .execucoes()
            .transicao_ja_aplicada(contexto.org, execucao.id, &chave)
            .map_err(|erro| falha_de_leitura(erro, "a chave do abandono"))?
        {
            return Ok("Este fluxo já tinha sido abandonado; nada mudou.".to_owned());
        }
        let carga =
            match fluxo::carga_para_agir(&self.ajustes, &self.store, contexto.org, &execucao) {
                Some(carga) => carga,
                None => return Err(nao_consegui_ler_o_fluxo()),
            };
        let resultado = motor::abandonar(&carga.estado, &motivo, AtorAcao::Agente, Some(&chave))
            .map_err(|erro| format!("erro: {erro}"))?;
        self.store
            .execucoes()
            .aplicar_transicao(
                contexto.org,
                execucao.id,
                execucao.versao,
                &transicao_do_motor(&resultado),
                &chave,
                ExtrasDaTransicao {
                    ator_ref: None,
                    fechada_por: None,
                    motivo_encerramento: None,
                },
            )
            .map_err(|erro| erro_da_transicao(erro, "abandonar"))?;
        Ok(format!(
            "Fluxo \"{}\" abandonado. A Ficha continua aberta; siga a conversa sem o roteiro.",
            carga.ir.fluxo.nome
        ))
    }
}

// ─────────────────────────────────────────────────────────────────── fluxo_iniciar

/// `fluxo_iniciar`: a ferramenta da reserva (Jev fora, gate desligado ou escolha recusada).
struct Iniciar(FluxoDoTurno);

impl FerramentaInterna for Iniciar {
    fn nome(&self) -> &'static str {
        NOME_INICIAR
    }

    fn descricao(&self) -> &'static str {
        "Inicia um fluxo de atendimento liberado para você, pelo id que aparece na lista \
         <fluxos_de_atendimento>. Use só quando a mensagem do cliente pedir claramente um dos \
         fluxos; depois de iniciar, siga a etapa que a resposta indicar."
    }

    fn parametros(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "fluxo_id": {
                    "type": "string",
                    "description": "Id do fluxo, exatamente como aparece em <fluxos_de_atendimento>."
                }
            },
            "required": ["fluxo_id"],
            "additionalProperties": false
        })
    }

    fn executar(&self, contexto: ContextoFerramenta, argumentos: Value) -> FuturoFerramenta {
        let comum = self.0.clone();
        Box::pin(async move { comum.executar_inicio(contexto, argumentos) })
    }
}

impl FluxoDoTurno {
    /// Valida como o G9 e abre a execução, com o evento assinado pelo ator `agente`.
    fn executar_inicio(
        &self,
        contexto: ContextoFerramenta,
        argumentos: Value,
    ) -> Result<String, String> {
        let pedido = texto_obrigatorio(&argumentos, "fluxo_id")?;
        let ficha = self
            .store
            .fichas()
            .por_conversa(contexto.org, contexto.conversa)
            .map_err(|erro| falha_de_leitura(erro, "a Ficha do atendimento"))?
            .ok_or_else(|| {
                "erro: este atendimento ainda não tem Ficha; não há fluxo para iniciar".to_owned()
            })?;
        if ficha.status == StatusFicha::Fechada {
            return Err(
                "erro: o atendimento já foi encerrado; a próxima mensagem do cliente abre outro"
                    .to_owned(),
            );
        }
        let slug = self.slug_do_pedido(contexto.org, ficha.agent_id, &pedido)?;
        match fluxo::iniciar(
            &self.ajustes,
            &self.store,
            contexto.org,
            &ficha,
            &slug,
            QuemInicia::Agente,
            None,
        )
        .map_err(|erro| falha_de_leitura(erro, "a abertura do fluxo"))?
        {
            fluxo::Inicio::Aberta { execucao, efeitos } => {
                let Some(carga) =
                    fluxo::carga_para_agir(&self.ajustes, &self.store, contexto.org, &execucao)
                else {
                    return Ok(format!(
                        "Fluxo `{slug}` iniciado. O bloco da etapa entra no próximo turno."
                    ));
                };
                let mut resposta = format!(
                    "Fluxo \"{}\" iniciado.\n{}",
                    carga.ir.fluxo.nome,
                    descrever(&carga.ir, &carga.estado)
                );
                if efeitos > 0 {
                    resposta.push_str(&nota_de_efeito());
                }
                Ok(resposta)
            }
            fluxo::Inicio::JaAtiva => Err(
                "erro: já existe um fluxo em andamento neste atendimento; continue nele".to_owned(),
            ),
            fluxo::Inicio::Recusado(motivo) => Err(format!("erro: {motivo}")),
        }
    }

    /// O slug do fluxo a partir do que o modelo escreveu: id, nome ou variação de caixa.
    ///
    /// O catálogo mostra o **id** (`slug`), mas o modelo pode repetir o nome; resolver aqui evita
    /// uma recusa boba. Fluxo que não está liberado ao agente não entra na lista, então ele nem é
    /// resolvido — a recusa sai como "não existe", sem revelar fluxo de outro agente.
    fn slug_do_pedido(
        &self,
        org: poph_types::OrgId,
        agente: poph_types::AgentId,
        pedido: &str,
    ) -> Result<String, String> {
        let pedido = pedido.trim();
        let liberados = self
            .store
            .fluxos()
            .listar_do_agente(org, agente)
            .map_err(|erro| falha_de_leitura(erro, "os fluxos liberados"))?;
        if let Some(fluxo) = liberados.iter().find(|fluxo| fluxo.slug == pedido) {
            return Ok(fluxo.slug.clone());
        }
        let minusculo = pedido.to_lowercase();
        let achado = liberados.iter().find(|fluxo| {
            fluxo.slug.to_lowercase() == minusculo || fluxo.nome.to_lowercase() == minusculo
        });
        match achado {
            Some(fluxo) => Ok(fluxo.slug.clone()),
            None => Err(format!(
                "erro: o fluxo `{pedido}` não está entre os liberados para este atendimento; \
                 use um id de <fluxos_de_atendimento>"
            )),
        }
    }
}

// ─────────────────────────────────────────────────────────────── texto para o modelo

/// Nota que acompanha uma ação que enfileirou efeito (o texto é o mesmo do bloco da etapa).
fn nota_de_efeito() -> String {
    format!("\n{}", fluxo::NOTA_DE_EFEITO)
}

/// O que o modelo lê depois de uma ação: se avançou, onde parou, o que falta e o que pode fazer.
fn resposta_da_acao(
    ir: &Ir,
    anterior: &EstadoDoMotor,
    resultado: &motor::Resultado,
    repetida: bool,
) -> String {
    let mut saida = String::new();
    if repetida {
        saida.push_str("Esta ação já tinha sido registrada; nada mudou.\n");
    }
    let novo = &resultado.novo_estado;
    let avancou = novo.no_atual.id() != anterior.no_atual.id();
    match &novo.no_atual {
        Destino::Etapa { .. } if avancou => {
            saida.push_str(&format!(
                "Ação registrada: o fluxo \"{}\" avançou.\n",
                ir.fluxo.nome
            ));
        }
        Destino::Etapa { .. } => {
            saida.push_str(&format!(
                "Ação registrada: o fluxo \"{}\" continua na mesma etapa.\n",
                ir.fluxo.nome
            ));
        }
        Destino::Fim { .. } | Destino::Decisao { .. } => {
            saida.push_str(&format!(
                "Ação registrada: o fluxo \"{}\" chegou ao desfecho.\n",
                ir.fluxo.nome
            ));
        }
    }
    saida.push_str(&descrever(ir, novo));
    if !resultado.efeitos.is_empty() {
        saida.push_str(&nota_de_efeito());
    }
    saida
}

/// Descrição do estado da execução em linguagem de atendimento, sem id interno de banco.
///
/// "Quem ainda precisa agir" sai do mesmo cálculo do motor (`atores_exigidos` menos os que já
/// agiram nesta etapa), então a resposta ao modelo e o critério `auto` não divergem.
fn descrever(ir: &Ir, estado: &EstadoDoMotor) -> String {
    let mut saida = String::new();
    match &estado.no_atual {
        Destino::Etapa { id } => {
            let Some(etapa) = ir.etapas.iter().find(|etapa| &etapa.id == id) else {
                return format!(
                    "Fluxo \"{}\": a etapa atual não existe na revisão publicada.",
                    ir.fluxo.nome
                );
            };
            saida.push_str(&format!("Etapa atual: \"{}\".", etapa.nome));
            if let Some(instrucao) = etapa
                .instrucao
                .as_deref()
                .map(str::trim)
                .filter(|texto| !texto.is_empty())
            {
                saida.push_str(&format!("\nOrientação da etapa: {instrucao}"));
            }
            let faltando = motor::campos_faltando(etapa, &estado.dados);
            saida.push_str(&format!(
                "\nCampos obrigatórios que faltam: {}.",
                lista(&faltando)
            ));
            let mut aguardando = motor::atores_exigidos(etapa);
            aguardando.retain(|ator| !estado.atores_que_agiram.contains(ator));
            if !aguardando.is_empty() {
                saida.push_str(&format!(
                    "\nQuem ainda precisa agir nesta etapa: {}.",
                    lista_dos_atores(&aguardando)
                ));
            }
            saida.push_str(&format!("\nAções válidas: {}.", lista_das_acoes(etapa)));
        }
        Destino::Decisao { .. } => {
            saida.push_str(
                "A execução está numa decisão do fluxo: o sistema resolve antes de qualquer ação.",
            );
        }
        Destino::Fim { resultado, .. } => {
            let desfecho = match resultado {
                Some(texto) => format!(" com o resultado \"{texto}\""),
                None => String::new(),
            };
            saida.push_str(&format!("A execução chegou ao fim{desfecho}."));
            if estado.status != poph_types::StatusExecucao::Ativa {
                saida.push_str(" Não há mais ação a executar neste fluxo.");
            }
        }
    }
    saida
}

/// Lista curta com "nenhum" quando vazia.
fn lista(itens: &[String]) -> String {
    if itens.is_empty() {
        return "nenhum".to_owned();
    }
    itens
        .iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Atores como o atendente lê.
fn lista_dos_atores(atores: &[AtorAcao]) -> String {
    atores
        .iter()
        .map(|ator| ator.como_texto().to_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Ações da etapa com id e rótulo (`seguir ("Seguir")`), que é o que o modelo precisa repetir.
fn lista_das_acoes(etapa: &poph_fluxos::ir::Etapa) -> String {
    if etapa.acoes.is_empty() {
        return "nenhuma (a etapa não tem saída)".to_owned();
    }
    etapa
        .acoes
        .iter()
        .map(|acao| format!("`{}` (\"{}\")", acao.id, acao.rotulo))
        .collect::<Vec<_>>()
        .join(", ")
}

// ───────────────────────────────────────────────────────────────────── argumentos

/// Texto obrigatório de um argumento, aparado; vazio é argumento inválido.
fn texto_obrigatorio(argumentos: &Value, campo: &str) -> Result<String, String> {
    argumentos
        .get(campo)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|texto| !texto.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("erro: o argumento `{campo}` é obrigatório e não pode ser vazio"))
}

/// `campos` opcional: objeto de valores; ausente ou `null` é objeto vazio.
fn campos_do_argumento(argumentos: &Value) -> Result<Map<String, Value>, String> {
    match argumentos.get("campos") {
        None | Some(Value::Null) => Ok(Map::new()),
        Some(Value::Object(mapa)) => Ok(mapa.clone()),
        Some(_) => {
            Err("erro: o argumento `campos` tem de ser um objeto {\"campo\": \"valor\"}".to_owned())
        }
    }
}

/// `chave_idempotencia` opcional: texto aparado; vazio conta como ausente.
fn chave_opcional(argumentos: &Value) -> Result<Option<String>, String> {
    match argumentos.get("chave_idempotencia") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(texto)) if texto.trim().is_empty() => Ok(None),
        Some(Value::String(texto)) => Ok(Some(texto.trim().to_owned())),
        Some(_) => Err("erro: o argumento `chave_idempotencia` tem de ser texto".to_owned()),
    }
}

/// Chave derivada de uma chamada sem `chave_idempotencia`.
///
/// É determinística pelo que a chamada tem de único: execução, nó, ação e campos. Assim repetir a
/// **mesma** chamada na mesma etapa devolve o desfecho anterior (o banco reconhece a marca) e a
/// mesma ação numa etapa seguinte, que é legítima, tem outra chave. O hash é o do processo: se a
/// versão do Rust mudar, no pior caso a repetição não é reconhecida e o motor recusa a ação que já
/// não existe na etapa — nunca grava efeito duplicado, porque a chave do efeito é do motor.
fn chave_derivada(execucao: &Execucao, no_atual: &str, acao: &str, campos: &Value) -> String {
    let mut hasher = DefaultHasher::new();
    campos.to_string().hash(&mut hasher);
    format!(
        "agente:{}:{no_atual}:{acao}:{:016x}",
        execucao.id,
        hasher.finish()
    )
}

// ─────────────────────────────────────────────────────────────────────── recusas

/// Falha de armazenamento: o detalhe vai para o log, o modelo recebe o caminho de saída.
fn falha_de_leitura(erro: ErroStore, o_que: &str) -> String {
    tracing::error!(erro = %erro, o_que, "ferramenta do fluxo falhou no armazenamento");
    format!("erro: não consegui ler {o_que} agora; siga a conversa sem o fluxo nesta mensagem")
}

/// A gravação recusou a transição: versão velha é corrida com o atendente, o resto é defeito.
fn erro_da_transicao(erro: ErroStore, acao: &str) -> String {
    match erro {
        ErroStore::Dominio(poph_types::DomainError::ConflitoDeVersao { .. }) => format!(
            "erro: outra pessoa agiu neste fluxo ao mesmo tempo e a etapa mudou; a ação `{acao}` \
             não foi gravada"
        ),
        ErroStore::Dominio(dominio) => format!("erro: {dominio}"),
        outro => falha_de_leitura(outro, "a gravação do fluxo"),
    }
}

/// A revisão da execução não carrega (IR divergente do texto): falha fechada, sem chutar etapa.
fn nao_consegui_ler_o_fluxo() -> String {
    "erro: não consegui ler a etapa atual deste fluxo; siga a conversa sem ele nesta mensagem"
        .to_owned()
}

#[cfg(test)]
mod testes {
    use poph_types::{DomainError, ExecucaoId};

    use poph_store::ErroStore;

    use super::{chave_derivada, erro_da_transicao};

    /// Execução fictícia só para derivar a chave (o resto dos campos não entra nela).
    fn execucao() -> poph_store::Execucao {
        poph_store::Execucao {
            id: ExecucaoId::generate(),
            org_id: poph_types::OrgId::generate(),
            ficha_id: poph_types::FichaId::generate(),
            fluxo_id: poph_types::FluxoId::generate(),
            revisao_id: poph_types::RevisaoId::generate(),
            status: poph_types::StatusExecucao::Ativa,
            no_atual: "coleta".to_owned(),
            dados_json: "{}".to_owned(),
            resultado: None,
            motivo_fim: None,
            iniciada_em: 0,
            finalizada_em: None,
            versao: 1,
        }
    }

    /// A chave derivada é estável no que identifica a chamada e muda quando a chamada muda: repetir
    /// a mesma ação na mesma etapa é reconhecido como repetição; a mesma ação noutra etapa (ou com
    /// outros campos) é uma chamada nova.
    #[test]
    fn chave_derivada_e_estavel_e_muda_com_a_chamada() {
        let execucao = execucao();
        let campos = serde_json::json!({ "nome": "Ana Teste" });
        let primeira = chave_derivada(&execucao, "coleta", "seguir", &campos);
        assert_eq!(
            primeira,
            chave_derivada(&execucao, "coleta", "seguir", &campos),
            "a mesma chamada tem a mesma chave"
        );
        assert_ne!(
            primeira,
            chave_derivada(
                &execucao,
                "coleta",
                "seguir",
                &serde_json::json!({"nome": "Bia"})
            ),
            "outro valor de campo é outra chamada"
        );
        assert_ne!(
            primeira,
            chave_derivada(&execucao, "confirmacao", "seguir", &campos),
            "a mesma ação noutra etapa é outra chamada"
        );
        assert!(primeira.starts_with("agente:"), "{primeira}");
    }

    /// Conflito de versão na gravação é corrida com o atendente, e a resposta ao modelo diz isso —
    /// não "erro de armazenamento".
    #[test]
    fn conflito_de_versao_vira_aviso_de_corrida() {
        let erro = ErroStore::Dominio(DomainError::ConflitoDeVersao {
            entidade: "execução de fluxo",
            id: "execucao-ficticia".to_owned(),
            esperada: 1,
            atual: 2,
        });
        let texto = erro_da_transicao(erro, "seguir");
        assert!(texto.contains("outra pessoa agiu"), "{texto}");
        assert!(texto.contains("`seguir`"), "{texto}");
    }
}
