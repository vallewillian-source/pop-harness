//! Entidades como o banco as devolve.
//!
//! Os campos ficam em português (como no resto do workspace) e as colunas em inglês (como no
//! PRD §7.2 e no POP Atende); o mapeamento coluna → campo está em cada repositório.
//! Nenhuma regra de negócio aqui: só a forma do dado.

use poph_types::{
    AgentId, AtorEvento, ConversationId, DepartmentId, EventoFichaId, ExecucaoId, FechadaPor,
    FichaId, FileId, IntegrationId, KnowledgeGroupId, KnowledgeItemId, MessageId, OrgId,
    ProductCategoryId, ProductId, StatusFicha, TipoEventoFicha, TurnId,
};

use crate::erro::{ErroStore, Resultado};

/// Situação de org, categoria, departamento, agente, grupo e item de conhecimento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Em uso.
    Ativo,
    /// Existe, mas fora do índice/busca e sem abrir turno novo (decisão D-05).
    Inativo,
    /// Guardado para histórico; o mesmo que inativo para o motor.
    Arquivado,
}

impl Status {
    /// Texto gravado na coluna `status`.
    pub fn como_texto(self) -> &'static str {
        match self {
            Status::Ativo => "active",
            Status::Inativo => "inactive",
            Status::Arquivado => "archived",
        }
    }

    /// Lê o texto da coluna `status`.
    pub fn de_texto(valor: &str) -> Resultado<Self> {
        match valor {
            "active" => Ok(Status::Ativo),
            "inactive" => Ok(Status::Inativo),
            "archived" => Ok(Status::Arquivado),
            outro => Err(ErroStore::Interno(format!(
                "status desconhecido no banco: `{outro}`"
            ))),
        }
    }

    /// `true` para o que participa do motor (ativo).
    pub fn esta_ativo(self) -> bool {
        self == Status::Ativo
    }
}

/// Página de listagem: cursor pelo id (UUID v7 é ordenável por tempo) e limite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pagina {
    /// Último id da página anterior (exclusivo). `None` começa do início.
    pub apos: Option<uuid::Uuid>,
    /// Quantos registros trazer. Padrão 50, teto 200.
    pub limite: u32,
}

impl Pagina {
    /// Página com limite (1 a 200).
    pub fn com_limite(limite: u32) -> Self {
        Self {
            apos: None,
            limite: limite.clamp(1, 200),
        }
    }
}

impl Default for Pagina {
    fn default() -> Self {
        Self::com_limite(50)
    }
}

// ───────────────────────────────── Org ─────────────────────────────────

/// Organização (tenant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Org {
    /// Id.
    pub id: OrgId,
    /// Nome de exibição.
    pub nome: String,
    /// Identificador na URL.
    pub slug: String,
    /// Situação.
    pub status: Status,
    /// Retenção, limites e padrões de modelo (JSON).
    pub config_json: String,
    /// Versão para `If-Match`.
    pub versao: u64,
    /// Criação (ms desde a época Unix).
    pub criado_em: i64,
    /// Última alteração (ms).
    pub atualizado_em: i64,
}

/// Dados para criar uma org.
#[derive(Debug, Clone, Default)]
pub struct NovaOrg {
    /// Nome de exibição.
    pub nome: String,
    /// Identificador na URL (`^[a-z0-9-]{3,64}$`, único).
    pub slug: String,
    /// Configuração da org em JSON (vazio = `{}`).
    pub config_json: String,
}

/// Campos que podem mudar numa org (o resto fica como está).
#[derive(Debug, Clone, Default)]
pub struct MudancaOrg {
    /// Novo nome.
    pub nome: Option<String>,
    /// Nova situação.
    pub status: Option<Status>,
    /// Nova configuração em JSON.
    pub config_json: Option<String>,
}

// ───────────────────────────────── Token de administração ─────────────────────────────────

/// Token de acesso à API administrativa (de org; os globais ficam só na configuração).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenAdmin {
    /// Id.
    pub id: poph_types::AdminTokenId,
    /// Org dona; `None` = global.
    pub org_id: Option<OrgId>,
    /// Rótulo para reconhecer o token na lista.
    pub rotulo: String,
    /// Hash do token (nunca o valor).
    pub hash: String,
    /// Criação (ms).
    pub criado_em: i64,
    /// Revogação (ms), quando houver.
    pub revogado_em: Option<i64>,
}

// ───────────────────────────────── Categoria de produto ─────────────────────────────────

/// Categoria que agrupa produtos e é vinculada a agentes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoriaProduto {
    /// Id.
    pub id: ProductCategoryId,
    /// Org dona.
    pub org_id: OrgId,
    /// Nome.
    pub nome: String,
    /// Descrição.
    pub descricao: String,
    /// Situação.
    pub status: Status,
    /// Versão para `If-Match`.
    pub versao: u64,
    /// Criação (ms).
    pub criado_em: i64,
    /// Última alteração (ms).
    pub atualizado_em: i64,
}

/// Dados para criar uma categoria.
#[derive(Debug, Clone, Default)]
pub struct NovaCategoria {
    /// Nome.
    pub nome: String,
    /// Descrição.
    pub descricao: String,
}

/// Campos que podem mudar numa categoria.
#[derive(Debug, Clone, Default)]
pub struct MudancaCategoria {
    /// Novo nome.
    pub nome: Option<String>,
    /// Nova descrição.
    pub descricao: Option<String>,
    /// Nova situação.
    pub status: Option<Status>,
}

/// Produto do catálogo (o que a ferramenta `buscar_produto` devolve).
///
/// O preço é **dado do registro**: fica aqui e nunca entra no índice de busca, então mudar o preço
/// aparece na resposta seguinte sem reindexar nada (HR-18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Produto {
    /// Id.
    pub id: ProductId,
    /// Org dona.
    pub org_id: OrgId,
    /// Categoria dona.
    pub categoria_id: ProductCategoryId,
    /// Nome comercial.
    pub nome: String,
    /// Identificador de URL (único por org).
    pub slug: String,
    /// Outros nomes pelos quais o cliente chama o produto.
    pub aliases: Vec<String>,
    /// Descrição.
    pub descricao: String,
    /// Preço como o cliente vê (texto livre, ex.: "R$ 320,00").
    pub preco_texto: String,
    /// Preço em centavos (exato; é o que a resposta usa).
    pub preco_centavos: i64,
    /// Unidade ("por m²", "por visita").
    pub unidade: String,
    /// O que está incluso.
    pub inclui: String,
    /// O que não está incluso.
    pub exclui: String,
    /// Condições (prazo, pagamento).
    pub condicoes: String,
    /// Etiquetas de busca.
    pub tags: Vec<String>,
    /// Situação: inativo/arquivado fica fora da busca e da ferramenta (D-05 aplicado a produto).
    pub status: Status,
    /// Versão para `If-Match`.
    pub versao: u64,
    /// Criação (ms).
    pub criado_em: i64,
    /// Última alteração (ms).
    pub atualizado_em: i64,
}

/// Dados para criar um produto.
#[derive(Debug, Clone, Default)]
pub struct NovoProduto {
    /// Categoria dona (obrigatória na criação).
    pub categoria_id: Option<ProductCategoryId>,
    /// Nome.
    pub nome: String,
    /// Slug. Vazio deriva do nome.
    pub slug: Option<String>,
    /// Aliases.
    pub aliases: Vec<String>,
    /// Descrição.
    pub descricao: String,
    /// Preço como o cliente vê.
    pub preco_texto: String,
    /// Preço em centavos.
    pub preco_centavos: i64,
    /// Unidade.
    pub unidade: String,
    /// Inclui.
    pub inclui: String,
    /// Exclui.
    pub exclui: String,
    /// Condições.
    pub condicoes: String,
    /// Tags.
    pub tags: Vec<String>,
}

/// Campos que podem mudar num produto. `None` = não mexer (mesma regra da HR-07).
#[derive(Debug, Clone, Default)]
pub struct MudancaProduto {
    /// Nova categoria.
    pub categoria_id: Option<ProductCategoryId>,
    /// Novo nome.
    pub nome: Option<String>,
    /// Novo slug.
    pub slug: Option<String>,
    /// Novos aliases (substitui a lista).
    pub aliases: Option<Vec<String>>,
    /// Nova descrição.
    pub descricao: Option<String>,
    /// Novo preço como o cliente vê.
    pub preco_texto: Option<String>,
    /// Novo preço em centavos.
    pub preco_centavos: Option<i64>,
    /// Nova unidade.
    pub unidade: Option<String>,
    /// Novo inclui.
    pub inclui: Option<String>,
    /// Novo exclui.
    pub exclui: Option<String>,
    /// Novas condições.
    pub condicoes: Option<String>,
    /// Novas tags (substitui a lista).
    pub tags: Option<Vec<String>>,
    /// Nova situação.
    pub status: Option<Status>,
}

/// Produto como ele entra no índice de busca (só o que casa por texto; preço não entra).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProdutoParaIndice {
    /// Id do produto.
    pub id: ProductId,
    /// Nome (a "fonte" mostrada ao modelo).
    pub nome: String,
    /// Nome da categoria.
    pub categoria: String,
    /// Texto a indexar: nome, aliases, descrição e tags. **Sem preço nem condições.**
    pub texto: String,
    /// Versão no `harness.db`.
    pub versao: u64,
}

/// Slug a partir de um texto livre: minúsculas, sem acento, palavras separadas por um traço.
pub fn slugificar(nome: &str) -> String {
    let mut slug = String::with_capacity(nome.len());
    let mut ultimo_traco = false;
    for caractere in nome.chars().flat_map(char::to_lowercase).map(sem_acento) {
        if caractere.is_ascii_alphanumeric() {
            slug.push(caractere);
            ultimo_traco = false;
        } else if !ultimo_traco && !slug.is_empty() {
            slug.push('-');
            ultimo_traco = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.truncate(64);
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

fn sem_acento(caractere: char) -> char {
    match caractere {
        'á' | 'à' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ç' => 'c',
        'ñ' => 'n',
        outro => outro,
    }
}

// ───────────────────────────────── Departamento ─────────────────────────────────

/// Departamento da org (destino de conversa e de transferência).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Departamento {
    /// Id.
    pub id: DepartmentId,
    /// Org dona.
    pub org_id: OrgId,
    /// Nome.
    pub nome: String,
    /// Descrição.
    pub descricao: String,
    /// Situação.
    pub status: Status,
    /// Versão para `If-Match`.
    pub versao: u64,
    /// Criação (ms).
    pub criado_em: i64,
    /// Última alteração (ms).
    pub atualizado_em: i64,
}

/// Dados para criar um departamento.
#[derive(Debug, Clone, Default)]
pub struct NovoDepartamento {
    /// Nome.
    pub nome: String,
    /// Descrição.
    pub descricao: String,
}

/// Campos que podem mudar num departamento.
#[derive(Debug, Clone, Default)]
pub struct MudancaDepartamento {
    /// Novo nome.
    pub nome: Option<String>,
    /// Nova descrição.
    pub descricao: Option<String>,
    /// Nova situação.
    pub status: Option<Status>,
}

// ───────────────────────────────── Agente ─────────────────────────────────

/// Agente (a configuração da PARIDADE_AGENTE.md que a V1 mantém).
#[derive(Debug, Clone, PartialEq)]
pub struct Agente {
    /// Id.
    pub id: AgentId,
    /// Org dona.
    pub org_id: OrgId,
    /// Nome.
    pub nome: String,
    /// Slug único por org.
    pub slug: String,
    /// Descrição.
    pub descricao: String,
    /// Situação (D-05: inativo não abre turno).
    pub status: Status,
    /// `custom` ou `standard` (paridade).
    pub agent_type: String,
    /// Categoria do POP Atende (paridade; sem efeito no motor).
    pub categoria: String,
    /// Departamento para onde vão as conversas novas.
    pub departamento_padrao: Option<DepartmentId>,
    /// Liga o gate G1 (transferência para humano).
    pub handoff_ativo: bool,
    /// Departamento humano (gate G4).
    pub handoff_departamento: Option<DepartmentId>,
    /// Provedor do LLM (`antigravity2api` na V1).
    pub provedor: String,
    /// Modelo do LLM (vale por agente).
    pub modelo: String,
    /// Temperatura (`None` = padrão do provedor).
    pub temperatura: Option<f64>,
    /// Teto de tokens de saída (`None` = padrão da configuração).
    pub max_tokens_saida: Option<i64>,
    /// Contexto em mensagens (paridade/importador; o motor orça por tokens).
    pub max_context_messages: u32,
    /// Orçamento de contexto em tokens.
    pub max_context_tokens: u32,
    /// Janela de agrupamento em ms (0 a 8000).
    pub janela_agrupamento_ms: u32,
    /// Máximo de rodadas de ferramenta por turno.
    pub max_rodadas_ferramenta: u32,
    /// Máximo de chamadas de ferramenta por mensagem.
    pub max_ferramentas_por_mensagem: u32,
    /// Quantos trechos o RAG devolve.
    pub rag_top_k: u32,
    /// Corte mínimo de relevância (0 a 1).
    pub rag_corte: f64,
    /// Flags e limiares do Jev deste agente (JSON; vazio = herda).
    pub jev_json: String,
    /// Versão para `If-Match`.
    pub versao: u64,
    /// Criação (ms).
    pub criado_em: i64,
    /// Última alteração (ms).
    pub atualizado_em: i64,
}

impl Agente {
    /// Decisão D-05: só agente `active` abre turno novo.
    ///
    /// A regra fica no tipo (e em [`repo::Agentes::obter_para_turno`]) de propósito: no POP Atende
    /// ela não existia e um agente `inactive` continuava respondendo no WhatsApp.
    pub fn pode_abrir_turno(&self) -> bool {
        self.status.esta_ativo()
    }
}

/// Dados para criar um agente. Os padrões seguem o PRD §6 e a PARIDADE_AGENTE.md.
#[derive(Debug, Clone)]
pub struct NovoAgente {
    /// Nome.
    pub nome: String,
    /// Slug único por org.
    pub slug: String,
    /// Descrição.
    pub descricao: String,
    /// `custom` (padrão) ou `standard`.
    pub agent_type: String,
    /// Categoria (paridade; padrão `generic`).
    pub categoria: String,
    /// Departamento padrão.
    pub departamento_padrao: Option<DepartmentId>,
    /// Provedor do LLM.
    pub provedor: String,
    /// Modelo do LLM.
    pub modelo: String,
    /// Temperatura.
    pub temperatura: Option<f64>,
    /// Teto de tokens de saída.
    pub max_tokens_saida: Option<i64>,
    /// Orçamento de contexto em tokens.
    pub max_context_tokens: u32,
    /// Janela de agrupamento em ms.
    pub janela_agrupamento_ms: u32,
    /// Máximo de rodadas de ferramenta.
    pub max_rodadas_ferramenta: u32,
    /// Máximo de chamadas de ferramenta por mensagem.
    pub max_ferramentas_por_mensagem: u32,
    /// Top-k do RAG.
    pub rag_top_k: u32,
    /// Corte mínimo do RAG.
    pub rag_corte: f64,
}

impl Default for NovoAgente {
    fn default() -> Self {
        Self {
            nome: String::new(),
            slug: String::new(),
            descricao: String::new(),
            agent_type: "custom".to_owned(),
            categoria: "generic".to_owned(),
            departamento_padrao: None,
            provedor: "antigravity2api".to_owned(),
            modelo: "gemini-3.8-flash-tiered".to_owned(),
            temperatura: None,
            max_tokens_saida: None,
            max_context_tokens: 64_000,
            janela_agrupamento_ms: 2_000,
            max_rodadas_ferramenta: 4,
            max_ferramentas_por_mensagem: 10,
            rag_top_k: 3,
            rag_corte: 0.0,
        }
    }
}

/// Campos que podem mudar num agente (o resto fica como está).
#[derive(Debug, Clone, Default)]
pub struct MudancaAgente {
    /// Novo nome.
    pub nome: Option<String>,
    /// Nova descrição.
    pub descricao: Option<String>,
    /// Nova situação.
    pub status: Option<Status>,
    /// Novo departamento padrão.
    pub departamento_padrao: Option<DepartmentId>,
    /// Liga/desliga o handoff.
    pub handoff_ativo: Option<bool>,
    /// Novo departamento humano.
    pub handoff_departamento: Option<DepartmentId>,
    /// Novo provedor.
    pub provedor: Option<String>,
    /// Novo modelo.
    pub modelo: Option<String>,
    /// Nova temperatura.
    pub temperatura: Option<f64>,
    /// Novo teto de tokens de saída.
    pub max_tokens_saida: Option<i64>,
    /// Novo orçamento de contexto em tokens.
    pub max_context_tokens: Option<u32>,
    /// Nova janela de agrupamento.
    pub janela_agrupamento_ms: Option<u32>,
    /// Novo máximo de rodadas de ferramenta.
    pub max_rodadas_ferramenta: Option<u32>,
    /// Novo máximo de chamadas de ferramenta por mensagem.
    pub max_ferramentas_por_mensagem: Option<u32>,
    /// Novo top-k do RAG.
    pub rag_top_k: Option<u32>,
    /// Novo corte do RAG.
    pub rag_corte: Option<f64>,
    /// Novas flags do Jev (JSON).
    pub jev_json: Option<String>,
}

// ───────────────────────────────── Instruções do agente ─────────────────────────────────

/// Instruções estruturadas e texto composto do agente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instrucoes {
    /// Agente dono.
    pub agent_id: AgentId,
    /// Org dona.
    pub org_id: OrgId,
    /// `StructuredInstructions` em JSON.
    pub estruturado_json: String,
    /// Texto composto (prefixo estável do prompt).
    pub markdown: String,
    /// `true` quando o markdown ainda é derivado do JSON.
    pub derivado_do_json: bool,
    /// Versão para `If-Match`.
    pub versao: u64,
    /// Criação (ms).
    pub criado_em: i64,
    /// Última alteração (ms).
    pub atualizado_em: i64,
}

// ───────────────────────────────── Conversa e mensagem ─────────────────────────────────

/// Situação da conversa (PRD §7.2 e D12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusConversa {
    /// O agente responde.
    Aberta,
    /// Transferida para humano: grava mensagem, não responde até reabrir.
    Transferida,
    /// Encerrada.
    Fechada,
}

impl StatusConversa {
    /// Texto gravado na coluna `status`.
    pub fn como_texto(self) -> &'static str {
        match self {
            StatusConversa::Aberta => "open",
            StatusConversa::Transferida => "transferred",
            StatusConversa::Fechada => "closed",
        }
    }

    /// Lê o texto da coluna `status`.
    pub fn de_texto(valor: &str) -> Resultado<Self> {
        match valor {
            "open" => Ok(StatusConversa::Aberta),
            "transferred" => Ok(StatusConversa::Transferida),
            "closed" => Ok(StatusConversa::Fechada),
            outro => Err(ErroStore::Interno(format!(
                "status de conversa desconhecido no banco: `{outro}`"
            ))),
        }
    }
}

/// Conversa entre um cliente e um agente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversa {
    /// Id.
    pub id: ConversationId,
    /// Org dona.
    pub org_id: OrgId,
    /// Agente que atende.
    pub agent_id: AgentId,
    /// Integração de origem (`None` no console/CLI).
    pub integration_id: Option<IntegrationId>,
    /// Canal (`whatsapp`, `console`, `cli`).
    pub canal: String,
    /// Identificador do cliente no canal.
    pub peer_externo: Option<String>,
    /// Situação.
    pub status: StatusConversa,
    /// Departamento alvo da transferência.
    pub departamento_alvo: Option<DepartmentId>,
    /// Resumo rolante (HR-14).
    pub resumo: Option<String>,
    /// Versão para `If-Match`.
    pub versao: u64,
    /// Criação (ms).
    pub criado_em: i64,
    /// Última alteração (ms).
    pub atualizado_em: i64,
    /// Última mensagem (ms).
    pub ultima_mensagem_em: Option<i64>,
    /// Nome do cliente no WhatsApp (`pushName`, já normalizado), quando um webhook já trouxe.
    pub nome_cliente: Option<String>,
}

/// Dados para abrir uma conversa.
#[derive(Debug, Clone)]
pub struct NovaConversa {
    /// Agente que atende.
    pub agent_id: AgentId,
    /// Integração de origem.
    pub integration_id: Option<IntegrationId>,
    /// Canal.
    pub canal: String,
    /// Identificador do cliente no canal.
    pub peer_externo: Option<String>,
    /// Departamento alvo inicial (normalmente o padrão do agente).
    pub departamento_alvo: Option<DepartmentId>,
}

/// Direção da mensagem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direcao {
    /// Do cliente para o agente.
    Entrada,
    /// Do agente para o cliente.
    Saida,
}

impl Direcao {
    /// Texto gravado na coluna `direction`.
    pub fn como_texto(self) -> &'static str {
        match self {
            Direcao::Entrada => "in",
            Direcao::Saida => "out",
        }
    }
}

/// Papel da mensagem no prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Papel {
    /// Cliente.
    Usuario,
    /// Agente.
    Assistente,
    /// Resultado de ferramenta.
    Ferramenta,
    /// Instrução de sistema.
    Sistema,
}

impl Papel {
    /// Texto gravado na coluna `role`.
    pub fn como_texto(self) -> &'static str {
        match self {
            Papel::Usuario => "user",
            Papel::Assistente => "assistant",
            Papel::Ferramenta => "tool",
            Papel::Sistema => "system",
        }
    }

    /// Lê o texto da coluna `role`.
    pub fn de_texto(valor: &str) -> Resultado<Self> {
        match valor {
            "user" => Ok(Papel::Usuario),
            "assistant" => Ok(Papel::Assistente),
            "tool" => Ok(Papel::Ferramenta),
            "system" => Ok(Papel::Sistema),
            outro => Err(ErroStore::Interno(format!(
                "papel de mensagem desconhecido no banco: `{outro}`"
            ))),
        }
    }
}

/// Quem produziu a mensagem (coluna `origin`, PRD do Matrix §3.2 P9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origem {
    /// O agente (turno). É o que já existia antes da coluna e o padrão do banco.
    Agente,
    /// Atendente humano, respondendo pelo Element.
    Humano,
    /// Digitada no celular do cliente, pelo WhatsApp (fase 2).
    Dispositivo,
}

impl Origem {
    /// Texto gravado na coluna `origin`.
    pub fn como_texto(self) -> &'static str {
        match self {
            Origem::Agente => "agent",
            Origem::Humano => "human",
            Origem::Dispositivo => "device",
        }
    }

    /// Lê o texto da coluna `origin`.
    pub fn de_texto(valor: &str) -> Resultado<Self> {
        match valor {
            "agent" => Ok(Origem::Agente),
            "human" => Ok(Origem::Humano),
            "device" => Ok(Origem::Dispositivo),
            outro => Err(ErroStore::Interno(format!(
                "origem de mensagem desconhecida no banco: `{outro}`"
            ))),
        }
    }
}

/// Mensagem de uma conversa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mensagem {
    /// Id.
    pub id: MessageId,
    /// Org dona.
    pub org_id: OrgId,
    /// Conversa dona.
    pub conversation_id: ConversationId,
    /// Direção.
    pub direcao: Direcao,
    /// Papel no prompt.
    pub papel: Papel,
    /// Quem produziu (`agent`, `human` ou `device`).
    pub origem: Origem,
    /// Nome do atendente, quando a origem é humana. `None` nas demais.
    pub autor_nome: Option<String>,
    /// Texto.
    pub texto: String,
    /// Anexos em JSON (ids de `attachments`).
    pub anexos_json: String,
    /// Id da mensagem no canal externo (idempotência).
    pub id_externo: Option<String>,
    /// Turno que a produziu/consumiu.
    pub turn_id: Option<TurnId>,
    /// Criação (ms).
    pub criado_em: i64,
}

/// Dados de uma resposta do atendente humano (MX-04), para
/// [`Mensagens::registrar_resposta_humana`](crate::Mensagens::registrar_resposta_humana).
#[derive(Debug, Clone, Copy)]
pub struct RespostaHumanaNova<'a> {
    /// Id do evento Matrix (idempotência: o mesmo id na mesma conversa não grava de novo).
    pub evento_id: &'a str,
    /// Corpo que o atendente escreveu (vai para `messages.text`).
    pub texto_bruto: &'a str,
    /// Texto já formatado que vai ao cliente (vai para a fila de saída).
    pub texto_enviado: &'a str,
    /// Id Matrix do atendente (`@ph_...:servidor`), vai para `actor_id` da auditoria.
    pub autor_id: &'a str,
    /// Nome do atendente, vai para `author_name`.
    pub autor_nome: &'a str,
}

/// O que foi gravado numa resposta humana aceita.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GravacaoHumana {
    /// Mensagem gravada (`origin = human`, com o nome do atendente).
    pub mensagem: Mensagem,
    /// Item da fila de saída, pendente, com o texto já formatado.
    pub saida: ItemSaida,
    /// `true` quando esta resposta passou a conversa de `open` para `transferred`.
    pub transferiu_agora: bool,
}

/// Resultado de [`Mensagens::registrar_resposta_humana`](crate::Mensagens::registrar_resposta_humana)
/// (MX-04): o que aconteceu com a resposta do atendente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RespostaHumana {
    /// Gravada: mensagem `human`, item na fila de saída e auditoria, tudo na mesma transação.
    Gravada(Box<GravacaoHumana>),
    /// Já existia mensagem com o mesmo id de evento nesta conversa: nada foi gravado de novo.
    Duplicada,
    /// A conversa está `closed`: nada foi gravado.
    ConversaFechada,
    /// Não existe conversa com esse id nesta org (inclusive quando ela é de outra org).
    ConversaInexistente,
}

/// Dados para gravar uma mensagem.
#[derive(Debug, Clone)]
pub struct NovaMensagem {
    /// Conversa dona.
    pub conversation_id: ConversationId,
    /// Direção.
    pub direcao: Direcao,
    /// Papel.
    pub papel: Papel,
    /// Quem produz: o turno grava `Origem::Agente`; só a resposta humana usa `Humano`.
    pub origem: Origem,
    /// Nome do atendente (só com `Origem::Humano`).
    pub autor_nome: Option<String>,
    /// Texto.
    pub texto: String,
    /// Anexos em JSON.
    pub anexos_json: String,
    /// Id externo (idempotência da entrada).
    pub id_externo: Option<String>,
    /// Turno.
    pub turn_id: Option<TurnId>,
}

// ───────────────────────────── Ficha do atendimento e eventos (FX-07) ─────────────────────────────

/// Ficha: o registro do atendimento (PRD §6.1).
///
/// Nasce junto com a conversa, na mesma escrita (`Conversas::criar` e a recepção do canal), e fecha
/// quando a conversa fecha. Os nomes de campo seguem as colunas do PRD §7.1, que nestas duas tabelas
/// são em português (`canal`, `peer_externo`, `aberta_em`), ao contrário do resto do banco.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ficha {
    /// Id.
    pub id: FichaId,
    /// Org dona.
    pub org_id: OrgId,
    /// Conversa que a Ficha registra. `None` quando a conversa foi apagada pela retenção (PRD L12).
    pub conversation_id: Option<ConversationId>,
    /// Agente que atendeu (retrato do instante em que a Ficha nasceu).
    pub agent_id: AgentId,
    /// Canal do atendimento.
    pub canal: String,
    /// Identificador do cliente no canal.
    pub peer_externo: Option<String>,
    /// Situação.
    pub status: StatusFicha,
    /// Abertura (ms).
    pub aberta_em: i64,
    /// Fechamento (ms). Só com `status = closed`.
    pub fechada_em: Option<i64>,
    /// Quem encerrou. Só com `status = closed`.
    pub fechada_por: Option<FechadaPor>,
    /// Por que encerrou, em texto legível.
    pub fechada_motivo: Option<String>,
    /// Versão para `If-Match`.
    pub versao: u64,
}

/// Dados para criar uma Ficha. A conversa é opcional porque a Ficha do backfill pode estar órfã,
/// mas o caminho normal passa sempre com a conversa recém-criada.
#[derive(Debug, Clone)]
pub struct NovaFicha {
    /// Conversa que a Ficha registra.
    pub conversation_id: Option<ConversationId>,
    /// Agente que atende.
    pub agent_id: AgentId,
    /// Canal do atendimento.
    pub canal: String,
    /// Identificador do cliente no canal.
    pub peer_externo: Option<String>,
}

/// Um evento da linha do tempo da Ficha (`ficha_eventos`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventoFicha {
    /// Id.
    pub id: EventoFichaId,
    /// Org dona.
    pub org_id: OrgId,
    /// Ficha dona.
    pub ficha_id: FichaId,
    /// Execução de fluxo a que o evento pertence, quando é evento de fluxo.
    pub execucao_id: Option<ExecucaoId>,
    /// Posição na linha do tempo da Ficha (1, 2, 3…). É o cursor da paginação.
    pub seq: i64,
    /// O que aconteceu.
    pub tipo: TipoEventoFicha,
    /// Quem assinou.
    pub ator_tipo: AtorEvento,
    /// Identificador de quem assinou (id Matrix do atendente, id do token), quando houver.
    pub ator_ref: Option<String>,
    /// Detalhe em JSON (campos gravados, ramo escolhido, erro do efeito).
    pub payload_json: String,
    /// Chave de idempotência: repetir a mesma operação não cria evento novo.
    pub chave_idempotencia: Option<String>,
    /// Quando aconteceu (ms).
    pub criado_em: i64,
}

/// Dados para registrar um evento da Ficha.
#[derive(Debug, Clone)]
pub struct NovoEventoFicha<'a> {
    /// Execução de fluxo a que o evento pertence, quando é evento de fluxo.
    pub execucao_id: Option<ExecucaoId>,
    /// O que aconteceu.
    pub tipo: TipoEventoFicha,
    /// Quem assinou.
    pub ator_tipo: AtorEvento,
    /// Identificador de quem assinou, quando houver.
    pub ator_ref: Option<&'a str>,
    /// Detalhe em JSON (`{}` quando não há).
    pub payload_json: String,
    /// Chave de idempotência (repetir com a mesma chave devolve o evento já gravado).
    pub chave_idempotencia: Option<&'a str>,
}

/// Filtros da lista de Fichas (PRD §7.3, `GET …/fichas`). Campo vazio não filtra.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltroFichas {
    /// Situação exata.
    pub status: Option<StatusFicha>,
    /// Agente que atendeu.
    pub agente: Option<AgentId>,
    /// Canal exato (`whatsapp`, `console`, `cli`).
    pub canal: Option<String>,
    /// Só Fichas abertas a partir deste instante (ms, inclusive).
    pub desde: Option<i64>,
}

// ───────────────────────────────── Caixa de entrada e de saída ─────────────────────────────────

/// Mensagem recebida, ainda não processada (PRD §6 passo 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemEntrada {
    /// Id.
    pub id: poph_types::InboxId,
    /// Org dona.
    pub org_id: OrgId,
    /// Integração de origem.
    pub integration_id: IntegrationId,
    /// Id da mensagem no canal.
    pub id_externo: String,
    /// Evento normalizado (`InboundEvent`) em JSON.
    pub evento_json: String,
    /// Recebimento (ms).
    pub recebido_em: i64,
    /// Processamento (ms), quando houver.
    pub processado_em: Option<i64>,
    /// Tentativas.
    pub tentativas: u32,
    /// Último erro.
    pub ultimo_erro: Option<String>,
}

/// Dados para gravar a entrada.
#[derive(Debug, Clone)]
pub struct NovaEntrada {
    /// Integração de origem.
    pub integration_id: IntegrationId,
    /// Id da mensagem no canal.
    pub id_externo: String,
    /// Evento normalizado em JSON.
    pub evento_json: String,
}

/// Resultado da gravação na entrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recebido {
    /// Gravado agora.
    Novo,
    /// Já estava lá (duplicata descartada, PRD §6 passo 1).
    Duplicado,
}

/// Dados para gravar a mensagem do cliente **e** o item da entrada na mesma transação.
///
/// É o que a integração usa: ou as duas linhas existem, ou nenhuma. Sem isso, uma queda entre a
/// gravação da mensagem e a da `inbox` deixaria a mensagem do cliente sem ninguém para processar.
#[derive(Debug, Clone)]
pub struct NovaEntradaComMensagem {
    /// Integração de origem.
    pub integration_id: IntegrationId,
    /// Id da mensagem no canal.
    pub id_externo: String,
    /// Evento normalizado em JSON.
    pub evento_json: String,
    /// Conversa que recebe a mensagem.
    pub conversation_id: ConversationId,
    /// Id da mensagem em `messages` (quem chama gera e põe no evento da `inbox`).
    pub mensagem_id: MessageId,
    /// Texto recebido (vazio é permitido: imagem sem legenda, por exemplo).
    pub texto: String,
    /// Anexos em JSON (ids de `attachments`).
    pub anexos_json: String,
}

/// Resultado da gravação atômica da entrada (item da `inbox` + mensagem do cliente).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecebidoComMensagem {
    /// Se a entrega era nova ou repetida.
    pub situacao: Recebido,
    /// Item da `inbox`.
    pub item: ItemEntrada,
    /// Mensagem gravada em `messages`.
    pub mensagem: Mensagem,
}

/// Estado de um item da saída (PRD §6 passo 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstadoSaida {
    /// Aguardando envio.
    Pendente,
    /// Entregue ao canal, sem confirmação.
    Enviando,
    /// Enviado.
    Enviado,
    /// Desistiu depois das tentativas.
    Falhou,
}

impl EstadoSaida {
    /// Texto gravado na coluna `state`.
    pub fn como_texto(self) -> &'static str {
        match self {
            EstadoSaida::Pendente => "pending",
            EstadoSaida::Enviando => "sending",
            EstadoSaida::Enviado => "sent",
            EstadoSaida::Falhou => "failed",
        }
    }

    /// Lê o texto da coluna `state`.
    pub fn de_texto(valor: &str) -> Resultado<Self> {
        match valor {
            "pending" => Ok(EstadoSaida::Pendente),
            "sending" => Ok(EstadoSaida::Enviando),
            "sent" => Ok(EstadoSaida::Enviado),
            "failed" => Ok(EstadoSaida::Falhou),
            outro => Err(ErroStore::Interno(format!(
                "estado de saída desconhecido no banco: `{outro}`"
            ))),
        }
    }
}

/// Item da fila de saída.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSaida {
    /// Id.
    pub id: poph_types::OutboxId,
    /// Org dona.
    pub org_id: OrgId,
    /// Conversa dona.
    pub conversation_id: ConversationId,
    /// Turno que gerou.
    pub turn_id: TurnId,
    /// Sequência dentro do turno.
    pub seq: u32,
    /// `text`, `file` ou `presence`.
    pub tipo: String,
    /// Conteúdo em JSON.
    pub conteudo_json: String,
    /// Estado.
    pub estado: EstadoSaida,
    /// Tentativas.
    pub tentativas: u32,
    /// Último erro.
    pub ultimo_erro: Option<String>,
    /// Criação (ms).
    pub criado_em: i64,
    /// Envio (ms).
    pub enviado_em: Option<i64>,
}

/// Dados para enfileirar uma saída.
#[derive(Debug, Clone)]
pub struct NovaSaida {
    /// Conversa dona.
    pub conversation_id: ConversationId,
    /// Turno que gerou.
    pub turn_id: TurnId,
    /// Sequência dentro do turno (idempotência com o turno).
    pub seq: u32,
    /// `text`, `file` ou `presence`.
    pub tipo: String,
    /// Conteúdo em JSON.
    pub conteudo_json: String,
}

// ───────────────────────────────── Turno ─────────────────────────────────

/// Estado do turno.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EstadoTurno {
    /// Rodando.
    Rodando,
    /// Terminou bem.
    Concluido,
    /// Terminou com erro.
    Falhou,
    /// Descartado (agente inativo, duplicata).
    Ignorado,
}

impl EstadoTurno {
    /// Texto gravado na coluna `state`.
    pub fn como_texto(self) -> &'static str {
        match self {
            EstadoTurno::Rodando => "running",
            EstadoTurno::Concluido => "done",
            EstadoTurno::Falhou => "failed",
            EstadoTurno::Ignorado => "skipped",
        }
    }

    /// Lê o texto da coluna `state`.
    pub fn de_texto(valor: &str) -> Resultado<Self> {
        match valor {
            "running" => Ok(EstadoTurno::Rodando),
            "done" => Ok(EstadoTurno::Concluido),
            "failed" => Ok(EstadoTurno::Falhou),
            "skipped" => Ok(EstadoTurno::Ignorado),
            outro => Err(ErroStore::Interno(format!(
                "estado de turno desconhecido no banco: `{outro}`"
            ))),
        }
    }
}

/// Registro de um turno (PRD §6 passo 9: o Console mostra o "por quê").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turno {
    /// Id.
    pub id: TurnId,
    /// Org dona.
    pub org_id: OrgId,
    /// Conversa.
    pub conversation_id: ConversationId,
    /// Agente.
    pub agent_id: Option<AgentId>,
    /// Estado.
    pub estado: EstadoTurno,
    /// Decisões do Jev (JSON).
    pub jev_json: String,
    /// Trechos recuperados (JSON).
    pub trechos_json: String,
    /// Ferramentas chamadas (JSON).
    pub ferramentas_json: String,
    /// Tempos por etapa (JSON).
    pub tempos_json: String,
    /// Tokens de entrada.
    pub tokens_entrada: u32,
    /// Tokens de saída.
    pub tokens_saida: u32,
    /// Custo em micros de unidade monetária.
    pub custo_micros: i64,
    /// Erro, quando falhou.
    pub erro: Option<String>,
    /// Início (ms).
    pub iniciado_em: i64,
    /// Fim (ms).
    pub finalizado_em: Option<i64>,
}

/// Registro de auditoria.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistroAuditoria {
    /// Id.
    pub id: poph_types::AuditId,
    /// Org (nulo para ação global).
    pub org_id: Option<OrgId>,
    /// Tipo de autor.
    pub ator_tipo: String,
    /// Id do autor.
    pub ator_id: Option<String>,
    /// Ação.
    pub acao: String,
    /// Entidade afetada.
    pub entidade: String,
    /// Id da entidade.
    pub entidade_id: Option<String>,
    /// Detalhe em JSON.
    pub detalhe_json: String,
    /// Quando (ms).
    pub criado_em: i64,
}

/// O que a finalização de um turno grava.
#[derive(Debug, Clone, Default)]
pub struct FinalizacaoTurno {
    /// Como terminou.
    pub estado: Option<EstadoTurno>,
    /// Decisões do Jev.
    pub jev_json: Option<String>,
    /// Trechos recuperados.
    pub trechos_json: Option<String>,
    /// Ferramentas chamadas.
    pub ferramentas_json: Option<String>,
    /// Tempos por etapa.
    pub tempos_json: Option<String>,
    /// Tokens de entrada.
    pub tokens_entrada: Option<u32>,
    /// Tokens de saída.
    pub tokens_saida: Option<u32>,
    /// Custo em micros.
    pub custo_micros: Option<i64>,
    /// Erro, quando falhou.
    pub erro: Option<String>,
}

impl FinalizacaoTurno {
    /// Finalização de sucesso, só com os tempos.
    pub fn concluido(tempos_json: &str) -> Self {
        Self {
            estado: Some(EstadoTurno::Concluido),
            tempos_json: Some(tempos_json.to_owned()),
            ..Self::default()
        }
    }

    /// Finalização de falha, com o motivo.
    pub fn falhou(erro: &str) -> Self {
        Self {
            estado: Some(EstadoTurno::Falhou),
            erro: Some(erro.to_owned()),
            ..Self::default()
        }
    }
}

/// Dados para registrar um evento de auditoria.
#[derive(Debug, Clone, Default)]
pub struct NovoRegistroAuditoria {
    /// Tipo do autor (`admin_token`, `system`, `plugin`, `cli`).
    pub ator_tipo: String,
    /// Id do autor (id do token, nome do subsistema).
    pub ator_id: Option<String>,
    /// Ação (`create`, `update`, `delete`, `transfer`, `retention`...).
    pub acao: String,
    /// Entidade afetada (`agent`, `conversation`...).
    pub entidade: String,
    /// Id da entidade.
    pub entidade_id: Option<String>,
    /// Detalhe em JSON.
    pub detalhe_json: String,
}

// ───────────────────────── Base de conhecimento (HR-17) ─────────────────────────

/// Limite de conteúdo de um item da base, em caracteres (PARIDADE_AGENTE.md §3, mantido do POP
/// Atende). O banco também tem o `CHECK`; a validação em código existe para a mensagem ser clara.
pub const LIMITE_CONTEUDO_ITEM: usize = 1000;

/// Grupo de itens da base de conhecimento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrupoConhecimento {
    /// Id.
    pub id: KnowledgeGroupId,
    /// Org dona.
    pub org_id: OrgId,
    /// Nome (único na org, sem diferenciar maiúsculas).
    pub nome: String,
    /// Descrição curta.
    pub descricao: String,
    /// Situação: grupo inativo sai do índice e da busca (D-05).
    pub status: Status,
    /// Versão para `If-Match`.
    pub versao: u64,
    /// Criação (ms).
    pub criado_em: i64,
    /// Última alteração (ms).
    pub atualizado_em: i64,
}

/// Dados para criar um grupo.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NovoGrupoConhecimento {
    /// Nome.
    pub nome: String,
    /// Descrição.
    pub descricao: String,
}

/// Campos alteráveis de um grupo; `None` não mexe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MudancaGrupoConhecimento {
    /// Novo nome.
    pub nome: Option<String>,
    /// Nova descrição.
    pub descricao: Option<String>,
    /// Nova situação.
    pub status: Option<Status>,
}

/// Item da base de conhecimento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemConhecimento {
    /// Id.
    pub id: KnowledgeItemId,
    /// Org dona.
    pub org_id: OrgId,
    /// Grupo ao qual pertence.
    pub grupo_id: KnowledgeGroupId,
    /// Título (vira a "fonte" mostrada ao modelo).
    pub titulo: String,
    /// Conteúdo (até [`LIMITE_CONTEUDO_ITEM`] caracteres).
    pub conteudo: String,
    /// Etiquetas livres.
    pub tags: Vec<String>,
    /// Posição dentro do grupo.
    pub posicao: i64,
    /// Situação: item inativo sai do índice e da busca (D-05).
    pub status: Status,
    /// Versão para `If-Match`.
    pub versao: u64,
    /// Criação (ms).
    pub criado_em: i64,
    /// Última alteração (ms).
    pub atualizado_em: i64,
}

/// Dados para criar um item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NovoItemConhecimento {
    /// Grupo dono.
    pub grupo_id: KnowledgeGroupId,
    /// Título.
    pub titulo: String,
    /// Conteúdo.
    pub conteudo: String,
    /// Etiquetas.
    pub tags: Vec<String>,
    /// Posição no grupo (0 quando omitida).
    pub posicao: i64,
}

/// Campos alteráveis de um item; `None` não mexe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MudancaItemConhecimento {
    /// Move para outro grupo.
    pub grupo_id: Option<KnowledgeGroupId>,
    /// Novo título.
    pub titulo: Option<String>,
    /// Novo conteúdo.
    pub conteudo: Option<String>,
    /// Novas etiquetas (substitui a lista inteira).
    pub tags: Option<Vec<String>>,
    /// Nova posição.
    pub posicao: Option<i64>,
    /// Nova situação.
    pub status: Option<Status>,
}

/// Item como o índice recebe: só o que está ativo, com o grupo resolvido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemParaIndice {
    /// Id do item.
    pub id: KnowledgeItemId,
    /// Grupo dono.
    pub grupo_id: KnowledgeGroupId,
    /// Nome do grupo (vai para o registro do turno, não para o texto embutido).
    pub grupo: String,
    /// Título.
    pub titulo: String,
    /// Conteúdo.
    pub conteudo: String,
    /// Versão do item no `harness.db`.
    pub versao: u64,
}

// ───────────────────── Biblioteca de arquivos do agente (HR-24) ─────────────────────

/// Arquivo da biblioteca de um agente: o que ele pode enviar ao cliente (D13).
///
/// O conteúdo não fica no SQLite: `blob_path` aponta para `<blobs>/<org>/<sha256>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArquivoBiblioteca {
    /// Id do arquivo.
    pub id: FileId,
    /// Org dona.
    pub org_id: OrgId,
    /// Agente dono (o arquivo só existe para ele).
    pub agent_id: AgentId,
    /// Nome mostrado ao cliente.
    pub nome: String,
    /// Descrição que o modelo lê para decidir quando enviar.
    pub descricao: String,
    /// Tipo MIME real, pelo conteúdo.
    pub mime: String,
    /// Tamanho em bytes.
    pub tamanho_bytes: u64,
    /// SHA-256 do conteúdo (deduplicação do blob).
    pub sha256: String,
    /// Caminho do blob relativo à pasta de blobs (`<org>/<sha256>`).
    pub blob_path: String,
    /// Criação (ms).
    pub criado_em: i64,
}

/// Dados para pôr um arquivo na biblioteca do agente.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NovoArquivoBiblioteca {
    /// Agente dono.
    pub agent_id: AgentId,
    /// Nome mostrado ao cliente.
    pub nome: String,
    /// Descrição para o modelo.
    pub descricao: String,
    /// MIME real.
    pub mime: String,
    /// Tamanho em bytes.
    pub tamanho_bytes: u64,
    /// SHA-256 do conteúdo.
    pub sha256: String,
    /// Caminho do blob relativo à pasta de blobs.
    pub blob_path: String,
}

// ───────────────────────── Entrada de canal (HR-26b) ─────────────────────────

/// Anexo recebido de um canal (WhatsApp), já gravado em blob pelo servidor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NovoAnexoCanal {
    /// Nome que o cliente deu ao arquivo (só o nome, sem pasta).
    pub nome: String,
    /// Tipo MIME real, pelo conteúdo.
    pub mime: String,
    /// Tamanho em bytes.
    pub tamanho_bytes: u64,
    /// SHA-256 em hexadecimal minúsculo.
    pub sha256: String,
    /// Caminho do blob relativo à pasta de blobs (`<org>/<sha256>`).
    pub blob_path: String,
    /// Texto extraído (documento com texto), quando houver.
    pub texto_extraido: Option<String>,
    /// Situação da extração: `ok`, `skipped` (imagem) ou `failed` (documento sem texto).
    pub status_extracao: &'static str,
}

/// Mensagem recebida de um canal, com o que é preciso para achar ou abrir a conversa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NovaEntradaCanal {
    /// Integração de origem (chave da idempotência junto com `id_externo`).
    pub integration_id: poph_types::IntegrationId,
    /// Agente da integração (dono da conversa, quando ela é aberta agora).
    pub agent_id: poph_types::AgentId,
    /// Canal (`whatsapp`).
    pub canal: String,
    /// Id da mensagem no canal (idempotência).
    pub id_externo: String,
    /// Identificador do cliente no canal (número, ou o grupo).
    pub peer: String,
    /// Evento normalizado em JSON, sem anexos embutidos.
    pub evento_json: String,
    /// Texto recebido (pode ser vazio, como imagem sem legenda).
    pub texto: String,
    /// Anexos já gravados em blob.
    pub anexos: Vec<NovoAnexoCanal>,
    /// Nome do cliente (`pushName`) desta mensagem, quando ela é de contato individual recebido
    /// (MX-32). Gravado na conversa dentro da mesma transação, num savepoint próprio: falha ao
    /// gravar o nome não derruba a mensagem.
    pub nome_cliente: Option<String>,
}

/// Anexo de uma mensagem, como está gravado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnexoRegistro {
    /// Id do anexo.
    pub id: poph_types::AttachmentId,
    /// Mensagem dona.
    pub mensagem_id: poph_types::MessageId,
    /// Nome que o cliente deu.
    pub nome: String,
    /// Tipo MIME.
    pub mime: String,
    /// Tamanho em bytes.
    pub tamanho_bytes: u64,
    /// SHA-256.
    pub sha256: String,
    /// Caminho do blob relativo à pasta de blobs.
    pub blob_path: String,
    /// Situação da extração de texto.
    pub status_extracao: String,
    /// Texto extraído do arquivo, já entre as marcas de conteúdo não confiável (HR-23).
    ///
    /// `None` quando não há texto (imagem, documento sem texto ou extração que falhou). É a leitura
    /// que o G7 (HR-35) usa para julgar injeção embutida no documento e a que o turno usa para
    /// entregar o arquivo ao modelo como dado, nunca instrução.
    pub texto_extraido: Option<String>,
}

/// Integração do canal, como está no banco (sem segredo e sem configuração).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegracaoDoCanal {
    /// Id da integração.
    pub id: poph_types::IntegrationId,
    /// Agente dono.
    pub agent_id: poph_types::AgentId,
    /// Tipo (`whatsapp`).
    pub kind: String,
    /// Situação (`active`, `inactive`, `archived`).
    pub status: String,
}
