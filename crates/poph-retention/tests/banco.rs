//! Testes da retenção (HR-51) sobre o banco real: `Store` numa pasta temporária, varredura pelo
//! adaptador `BancoHarness`. O relógio é injetado em `Opcoes::agora_ms`, então o teste simula o
//! futuro sem dormir.
//!
//! Os dados de teste (anexos, biblioteca, base, produtos) são semeados direto no `harness.db`,
//! porque o repositório desses itens ainda não existe. A varredura em si não usa SQL fora do
//! `poph-store`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use poph_retention::{
    BancoHarness, ErroRetencao, Opcoes, RelatorioOrg, Situacao, VarreduraBanco, executar_org,
    executar_todas,
};
use poph_store::{
    ExecutionStart, ExecutionState, NewConversation, NewFlow, RevisionPublication, Store,
    TransitionEffect,
};
use poph_types::{AgentId, EventActor, ExecutionStatus, OrgId};
use rusqlite::{Connection, params, types::Value};
use tempfile::TempDir;

const DIA: i64 = 86_400_000;
const HORA: i64 = 3_600_000;
/// "Agora" fixo: 19/11/2026.
const AGORA: i64 = 1_795_131_511_000;
/// Dados de "agora" (1 hora atrás): ficam em qualquer varredura com retenção de 1 dia ou mais.
const RECENTE: i64 = AGORA - HORA;

/// Tabelas com dados de uma org (exceto `audit_log`, que a retenção escreve de propósito).
const TABELAS_DA_ORG: &[&str] = &[
    "agents",
    "agent_instructions",
    "agent_product_categories",
    "departments",
    "product_categories",
    "products",
    "knowledge_groups",
    "knowledge_items",
    "agent_knowledge_groups",
    "file_library",
    "integrations",
    "conversations",
    "messages",
    "attachments",
    "carts",
    "cart_items",
    "inbox",
    "outbox",
    "turns",
];

/// Tabelas que a retenção nunca pode tocar (base, produtos, biblioteca, cadastro).
const TABELAS_PROTEGIDAS: &[&str] = &[
    "agents",
    "agent_instructions",
    "agent_product_categories",
    "departments",
    "product_categories",
    "products",
    "knowledge_groups",
    "knowledge_items",
    "agent_knowledge_groups",
    "file_library",
    "integrations",
];

/// Sha256 de teste: 64 hexadecimais minúsculos, do mesmo formato do blob real.
fn sha(n: u64) -> String {
    format!("{n:064x}")
}

fn novo_id() -> String {
    OrgId::generate().to_string()
}

/// Um `Store` numa pasta temporária. A pasta some no fim do teste.
struct Cenario {
    store: Store,
    harness: PathBuf,
    blobs: PathBuf,
    _pasta: TempDir,
}

impl Cenario {
    fn novo() -> Self {
        let pasta = TempDir::new().expect("pasta temporária");
        let dados = pasta.path().join("dados");
        let harness = dados.join("harness.db");
        let rag = dados.join("rag.db");
        let store = Store::open_at(&harness, &rag).expect("abre o banco de teste");
        Self {
            store,
            blobs: dados.join("blobs"),
            harness,
            _pasta: pasta,
        }
    }

    /// Conexão separada para semear e para conferir o banco (o `Store` guarda a dele).
    fn banco(&self) -> Connection {
        let conexao = Connection::open(&self.harness).expect("abre o banco");
        conexao
            .pragma_update(None, "foreign_keys", "ON")
            .expect("chaves estrangeiras");
        conexao
            .busy_timeout(Duration::from_secs(5))
            .expect("tempo de espera");
        conexao
    }

    fn armazem(&self) -> BancoHarness<'_> {
        BancoHarness::novo(&self.store, &self.blobs)
    }

    fn pasta_da_org(&self, org: &str) -> PathBuf {
        self.blobs.join(org)
    }
}

// ───────────────────────────── Semeadura ─────────────────────────────

fn org(c: &Connection, slug: &str, config: &str) -> String {
    let id = novo_id();
    c.execute(
        "INSERT INTO orgs (id, name, slug, status, config_json, version, created_at, updated_at)
         VALUES (?1, ?2, ?2, 'active', ?3, 1, ?4, ?4)",
        params![id, slug, config, AGORA - 400 * DIA],
    )
    .expect("org");
    id
}

fn agente(c: &Connection, org: &str) -> String {
    let id = novo_id();
    c.execute(
        "INSERT INTO agents (id, org_id, name, slug, provider, model, created_at, updated_at)
         VALUES (?1, ?2, 'Bia', ?3, 'antigravity2api', 'modelo-teste', ?4, ?4)",
        params![id, org, format!("bia-{id}"), AGORA - 400 * DIA],
    )
    .expect("agente");
    id
}

fn conversa(c: &Connection, org: &str, agente: &str, criada: i64) -> String {
    let id = novo_id();
    c.execute(
        "INSERT INTO conversations (id, org_id, agent_id, channel, status, version, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'cli', 'open', 1, ?4, ?4)",
        params![id, org, agente, criada],
    )
    .expect("conversa");
    id
}

fn mensagem(c: &Connection, org: &str, conversa: &str, criada: i64, turno: Option<&str>) -> String {
    let id = novo_id();
    c.execute(
        "INSERT INTO messages (id, org_id, conversation_id, direction, role, text, created_at, turn_id)
         VALUES (?1, ?2, ?3, 'in', 'user', 'oi', ?4, ?5)",
        params![id, org, conversa, criada, turno],
    )
    .expect("mensagem");
    id
}

fn anexo(c: &Connection, org: &str, mensagem: &str, sha256: &str, criado: i64) {
    c.execute(
        "INSERT INTO attachments (id, org_id, message_id, name, mime, size_bytes, sha256, blob_path, created_at)
         VALUES (?1, ?2, ?3, 'doc.pdf', 'application/pdf', 10, ?4, ?5, ?6)",
        params![novo_id(), org, mensagem, sha256, format!("{org}/{sha256}"), criado],
    )
    .expect("anexo");
}

fn turno(
    c: &Connection,
    org: &str,
    conversa: &str,
    agente: &str,
    estado: &str,
    iniciado: i64,
    terminado: Option<i64>,
) -> String {
    let id = novo_id();
    c.execute(
        "INSERT INTO turns (id, org_id, conversation_id, agent_id, state, started_at, finished_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![id, org, conversa, agente, estado, iniciado, terminado],
    )
    .expect("turno");
    id
}

fn saida(
    c: &Connection,
    org: &str,
    conversa: &str,
    turno: &str,
    estado: &str,
    criada: i64,
    enviada: Option<i64>,
) {
    let seq: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM outbox WHERE turn_id = ?1",
            params![turno],
            |linha| linha.get(0),
        )
        .expect("seq");
    c.execute(
        "INSERT INTO outbox (id, org_id, conversation_id, turn_id, seq, kind, payload_json, state, attempts, created_at, sent_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'text', '{}', ?6, 0, ?7, ?8)",
        params![novo_id(), org, conversa, turno, seq, estado, criada, enviada],
    )
    .expect("saída");
}

fn entrada(c: &Connection, org: &str, recebida: i64, processada: Option<i64>) {
    c.execute(
        "INSERT INTO inbox (id, org_id, integration_id, external_message_id, payload_json, received_at, processed_at, attempts)
         VALUES (?1, ?2, ?3, ?4, '{}', ?5, ?6, 0)",
        params![novo_id(), org, novo_id(), novo_id(), recebida, processada],
    )
    .expect("entrada");
}

fn biblioteca(c: &Connection, org: &str, agente: &str, sha256: &str) {
    c.execute(
        "INSERT INTO file_library (id, org_id, agent_id, name, mime, size_bytes, sha256, blob_path, created_at)
         VALUES (?1, ?2, ?3, 'tabela.xlsx', 'application/vnd.ms-excel', 10, ?4, ?5, ?6)",
        params![novo_id(), org, agente, sha256, format!("{org}/{sha256}"), AGORA - 400 * DIA],
    )
    .expect("biblioteca");
}

fn base_e_produtos(c: &Connection, org: &str) {
    let grupo = novo_id();
    c.execute(
        "INSERT INTO knowledge_groups (id, org_id, name, created_at, updated_at) VALUES (?1, ?2, 'Horários', ?3, ?3)",
        params![grupo, org, AGORA - 400 * DIA],
    )
    .expect("grupo da base");
    c.execute(
        "INSERT INTO knowledge_items (id, org_id, group_id, title, content, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'Seg a sex', 'Das 8h às 18h', ?4, ?4)",
        params![novo_id(), org, grupo, AGORA - 400 * DIA],
    )
    .expect("item da base");
    let categoria = novo_id();
    c.execute(
        "INSERT INTO product_categories (id, org_id, name, created_at, updated_at) VALUES (?1, ?2, 'Planos', ?3, ?3)",
        params![categoria, org, AGORA - 400 * DIA],
    )
    .expect("categoria");
    c.execute(
        "INSERT INTO products (id, org_id, category_id, name, slug, price_text, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'Plano A', ?4, 'R$ 10', ?5, ?5)",
        params![novo_id(), org, categoria, format!("plano-{categoria}"), AGORA - 400 * DIA],
    )
    .expect("produto");
}

/// Cenário principal. Com `idade_velha_dias` (ex.: 40), tudo "velho" tem essa idade; o recente
/// tem 1 hora. Devolve o id da org.
///
/// - conversa vazia e antiga: sai;
/// - conversa só com mensagens antigas (com um anexo): sai inteira;
/// - conversa mista (antigas + recentes): fica, só as antigas saem;
/// - turnos: 1 encerrado sem saída (sai); 1 rodando (fica); 1 com saída pendente (fica);
///   1 recente (fica);
/// - saída: 1 enviada e antiga (sai); pendente e com falha antigas (ficam); enviada recente (fica);
/// - entrada: 1 processada antiga (sai); 1 não processada antiga (fica); 1 processada recente (fica);
/// - biblioteca e base e produtos: sempre ficam.
fn semear(c: &Connection, slug: &str, config: &str, idade_velha_dias: i64) -> String {
    let velho = AGORA - idade_velha_dias * DIA;
    let org = org(c, slug, config);
    let ag = agente(c, &org);

    // Conversa sem nenhuma mensagem: é a conversa vazia que deve sair.
    conversa(c, &org, &ag, velho);
    let so_antigas = conversa(c, &org, &ag, velho);
    let mista = conversa(c, &org, &ag, velho);
    let recente = conversa(c, &org, &ag, RECENTE);

    let m1 = mensagem(c, &org, &so_antigas, velho, None);
    anexo(c, &org, &m1, &sha(1), velho);
    mensagem(c, &org, &so_antigas, velho + 1, None);
    let m3 = mensagem(c, &org, &mista, velho, None);
    anexo(c, &org, &m3, &sha(2), velho);
    let n1 = mensagem(c, &org, &mista, RECENTE, None);
    anexo(c, &org, &n1, &sha(3), RECENTE);
    mensagem(c, &org, &recente, RECENTE, None);

    let feito = turno(c, &org, &recente, &ag, "done", velho, Some(velho + 5));
    turno(c, &org, &recente, &ag, "running", velho, None);
    let pendente = turno(c, &org, &recente, &ag, "done", velho, Some(velho + 5));
    let do_dia = turno(c, &org, &recente, &ag, "done", RECENTE, Some(RECENTE + 5));
    saida(c, &org, &recente, &feito, "sent", velho, Some(velho + 5));
    saida(c, &org, &recente, &pendente, "pending", velho, None);
    saida(c, &org, &recente, &pendente, "failed", velho, None);
    saida(c, &org, &recente, &do_dia, "sent", RECENTE, Some(RECENTE));

    entrada(c, &org, velho, Some(velho + 1));
    entrada(c, &org, velho, None);
    entrada(c, &org, RECENTE, Some(RECENTE));

    biblioteca(c, &org, &ag, &sha(9));
    base_e_produtos(c, &org);
    org
}

// ───────────────────────────── Conferência ─────────────────────────────

/// Foto das tabelas da org, linha a linha (para comparar antes e depois).
fn foto(c: &Connection, org: &str, tabelas: &[&str]) -> BTreeMap<String, Vec<String>> {
    let mut mapa = BTreeMap::new();
    for tabela in tabelas {
        let mut consulta = c
            .prepare(&format!(
                "SELECT * FROM {tabela} WHERE org_id = ?1 ORDER BY rowid"
            ))
            .expect("consulta da foto");
        let colunas = consulta.column_count();
        let linhas = consulta
            .query_map(params![org], |linha| {
                let mut partes = Vec::with_capacity(colunas);
                for i in 0..colunas {
                    let valor: Value = linha.get(i)?;
                    partes.push(format!("{valor:?}"));
                }
                Ok(partes.join("|"))
            })
            .expect("foto");
        let linhas: Vec<String> = linhas.map(|linha| linha.expect("linha da foto")).collect();
        mapa.insert((*tabela).to_owned(), linhas);
    }
    mapa
}

fn contar(c: &Connection, tabela: &str, org: &str) -> i64 {
    c.query_row(
        &format!("SELECT COUNT(*) FROM {tabela} WHERE org_id = ?1"),
        params![org],
        |linha| linha.get(0),
    )
    .expect("contagem")
}

/// Detalhes das linhas de auditoria de retenção da org, do mais antigo para o mais novo.
fn auditorias(c: &Connection, org: &str) -> Vec<serde_json::Value> {
    let mut consulta = c
        .prepare(
            "SELECT detail_json FROM audit_log
              WHERE org_id = ?1 AND action = 'retention' ORDER BY created_at, id",
        )
        .expect("consulta da auditoria");
    let linhas = consulta
        .query_map(params![org], |linha| linha.get::<_, String>(0))
        .expect("auditoria");
    linhas
        .map(|linha| {
            let texto = linha.expect("linha da auditoria");
            serde_json::from_str(&texto).expect("detalhe em JSON")
        })
        .collect()
}

/// Conteúdo de todos os arquivos de uma pasta (nome → bytes). Pasta inexistente = vazia.
fn arquivos(pasta: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut mapa = BTreeMap::new();
    let Ok(entradas) = fs::read_dir(pasta) else {
        return mapa;
    };
    for entrada in entradas {
        let caminho = entrada.expect("entrada").path();
        if caminho.is_file() {
            let nome = caminho
                .file_name()
                .expect("nome")
                .to_string_lossy()
                .into_owned();
            mapa.insert(nome, fs::read(&caminho).expect("lê o arquivo"));
        }
    }
    mapa
}

fn gravar_arquivo(pasta: &Path, nome: &str, idade_ms: i64) {
    fs::create_dir_all(pasta).expect("pasta da org");
    let caminho = pasta.join(nome);
    fs::write(&caminho, nome.as_bytes()).expect("grava o arquivo");
    let arquivo = fs::OpenOptions::new()
        .write(true)
        .open(&caminho)
        .expect("abre o arquivo");
    arquivo
        .set_modified(UNIX_EPOCH + Duration::from_millis((AGORA - idade_ms) as u64))
        .expect("fixa a data do arquivo");
}

fn opcoes(padrao_dias: u32, simular: bool) -> Opcoes {
    Opcoes::novas(AGORA, padrao_dias, simular, "system")
}

fn org_id(texto: &str) -> OrgId {
    texto.parse().expect("id de org válido")
}

fn banco_da(relatorio: &RelatorioOrg) -> VarreduraBanco {
    relatorio
        .banco
        .clone()
        .expect("org varrida tem contagem do banco")
}

// ───────────────────────────── Testes ─────────────────────────────

#[test]
fn varre_so_o_vencido_com_contagem_exata() {
    let cenario = Cenario::novo();
    let c = cenario.banco();
    let org = semear(&c, "principal", "{}", 40);
    let armazem = cenario.armazem();

    let relatorio = executar_org(&armazem, org_id(&org), &opcoes(30, false)).expect("varredura");

    assert_eq!(
        relatorio.situacao,
        Situacao::Varrida,
        "{:?}",
        relatorio.motivo
    );
    let banco = banco_da(&relatorio);
    assert_eq!(banco.mensagens, 3, "mensagens antigas");
    assert_eq!(banco.anexos, 2, "anexos das mensagens antigas");
    assert_eq!(banco.turnos, 1, "só o turno encerrado sem saída pendente");
    assert_eq!(banco.outbox, 1, "só a saída enviada e antiga");
    assert_eq!(banco.inbox, 1, "só a entrada processada e antiga");
    assert_eq!(banco.conversas, 2, "a vazia e a que ficou sem mensagem");
    assert_eq!(banco.ids_conversas.len(), 3, "as três conversas afetadas");
    assert_eq!(
        banco.hashes_referenciados,
        vec![sha(3), sha(9)],
        "anexo vivo e biblioteca continuam com dono"
    );

    // O que sobrou no banco bate com o que ficou de fora.
    assert_eq!(contar(&c, "messages", &org), 2);
    assert_eq!(contar(&c, "attachments", &org), 1);
    assert_eq!(contar(&c, "conversations", &org), 2);
    assert_eq!(contar(&c, "turns", &org), 3, "rodando, pendente e recente");
    assert_eq!(
        contar(&c, "outbox", &org),
        3,
        "pendente, com falha e enviada recente"
    );
    assert_eq!(
        contar(&c, "inbox", &org),
        2,
        "não processada e processada recente"
    );

    // Segunda varredura no mesmo dia não apaga mais nada.
    let de_novo = executar_org(&armazem, org_id(&org), &opcoes(30, false)).expect("de novo");
    let banco2 = banco_da(&de_novo);
    assert_eq!(
        banco2.mensagens
            + banco2.anexos
            + banco2.turnos
            + banco2.inbox
            + banco2.outbox
            + banco2.conversas,
        0
    );
}

#[test]
fn retencao_zero_nao_apaga_nada_mesmo_com_dados_de_dez_anos() {
    let cenario = Cenario::novo();
    let c = cenario.banco();
    let org = semear(&c, "guardada", r#"{"retencao":{"dias":0}}"#, 3650);
    let antes = foto(&c, &org, TABELAS_DA_ORG);

    let relatorio =
        executar_org(&cenario.armazem(), org_id(&org), &opcoes(30, false)).expect("varredura");

    assert_eq!(relatorio.situacao, Situacao::Ignorada);
    assert_eq!(relatorio.banco, None);
    assert_eq!(foto(&c, &org, TABELAS_DA_ORG), antes, "nada mudou no banco");
    assert!(
        auditorias(&c, &org).is_empty(),
        "org pulada não gera auditoria"
    );
}

#[test]
fn base_produtos_e_biblioteca_ficam_intactos_com_a_varredura_mais_agressiva() {
    let cenario = Cenario::novo();
    let c = cenario.banco();
    // Retenção de 1 dia e dados de 400 dias: a varredura mais agressiva possível.
    let org = semear(&c, "agressiva", "{}", 400);
    let antes = foto(&c, &org, TABELAS_PROTEGIDAS);
    assert!(!antes["knowledge_items"].is_empty() && !antes["products"].is_empty());

    executar_org(&cenario.armazem(), org_id(&org), &opcoes(1, false)).expect("varredura");

    assert_eq!(
        foto(&c, &org, TABELAS_PROTEGIDAS),
        antes,
        "cadastro e biblioteca intactos"
    );
    assert!(
        contar(&c, "conversations", &org) >= 1,
        "a conversa recente continua"
    );
}

#[test]
fn simulacao_nao_altera_banco_nem_pasta_e_conta_o_que_sairia() {
    let cenario = Cenario::novo();
    let c = cenario.banco();
    let org = semear(&c, "simulada", "{}", 40);
    let pasta = cenario.pasta_da_org(&org);
    gravar_arquivo(&pasta, &sha(1), 40 * DIA);
    gravar_arquivo(&pasta, &sha(2), 40 * DIA);
    gravar_arquivo(&pasta, &sha(3), 40 * DIA);
    gravar_arquivo(&pasta, &sha(7), 40 * DIA);
    gravar_arquivo(&pasta, "pendente.tmp", 40 * DIA);
    let antes_banco = foto(&c, &org, TABELAS_DA_ORG);
    let antes_arquivos = arquivos(&pasta);
    let armazem = cenario.armazem();

    let simulado = executar_org(&armazem, org_id(&org), &opcoes(30, true)).expect("simulação");

    assert_eq!(simulado.situacao, Situacao::Varrida);
    assert_eq!(simulado.arquivos_apagados, 0);
    assert_eq!(
        foto(&c, &org, TABELAS_DA_ORG),
        antes_banco,
        "banco idêntico após a simulação"
    );
    assert_eq!(
        arquivos(&pasta),
        antes_arquivos,
        "pasta de blobs idêntica após a simulação"
    );
    let auditoria = auditorias(&c, &org);
    assert_eq!(
        auditoria.len(),
        1,
        "a simulação deixa uma linha de auditoria"
    );
    assert_eq!(auditoria[0]["simulacao"], true, "marcada como simulação");
    assert_eq!(
        auditoria[0]["mensagens"], 3,
        "a auditoria traz o que sairia"
    );

    // A execução real faz exatamente o que a simulação disse.
    let real = executar_org(&armazem, org_id(&org), &opcoes(30, false)).expect("real");
    assert_eq!(banco_da(&real), banco_da(&simulado), "mesmas contagens");
    assert_eq!(
        real.arquivos_sem_dono, simulado.arquivos_sem_dono,
        "mesmos arquivos órfãos"
    );
    assert_eq!(
        real.arquivos_apagados, 4,
        "três blobs órfãos e um temporário"
    );
    let restantes: Vec<String> = arquivos(&pasta).into_keys().collect();
    assert_eq!(restantes, vec![sha(3)], "só o blob vivo fica");
}

#[test]
fn blobs_orfaos_somem_referenciados_ficam_e_a_outra_org_nao_e_tocada() {
    let cenario = Cenario::novo();
    let c = cenario.banco();
    let a = semear(&c, "a", "{}", 40);
    let b = semear(&c, "b", "{}", 40);
    let pasta_a = cenario.pasta_da_org(&a);
    gravar_arquivo(&pasta_a, &sha(1), 40 * DIA); // órfão: o anexo que o usava vai sair
    gravar_arquivo(&pasta_a, &sha(3), 40 * DIA); // vivo: anexo recente
    gravar_arquivo(&pasta_a, &sha(9), 40 * DIA); // vivo: biblioteca do agente
    gravar_arquivo(&pasta_a, &sha(5), HORA / 2); // órfão mas recente: carência de 1 hora
    gravar_arquivo(&pasta_a, "sobra.tmp", 40 * DIA); // temporário abandonado
    gravar_arquivo(&pasta_a, "notas.txt", 40 * DIA); // não é blob: nunca toca
    let pasta_b = cenario.pasta_da_org(&b);
    gravar_arquivo(&pasta_b, &sha(4), 40 * DIA); // órfão da OUTRA org
    let banco_b_antes = foto(&c, &b, TABELAS_DA_ORG);
    let arquivos_b_antes = arquivos(&pasta_b);

    let relatorio =
        executar_org(&cenario.armazem(), org_id(&a), &opcoes(30, false)).expect("varredura");

    assert_eq!(relatorio.arquivos_apagados, 2);
    assert_eq!(
        relatorio.arquivos_sem_dono,
        vec![sha(1), "sobra.tmp".to_owned()]
    );
    let restantes: Vec<String> = arquivos(&pasta_a).into_keys().collect();
    assert_eq!(
        restantes,
        vec![sha(3), sha(5), sha(9), "notas.txt".to_owned()],
        "vivos, recente e não-blob ficam"
    );
    assert_eq!(
        arquivos(&pasta_b),
        arquivos_b_antes,
        "pasta da outra org intacta"
    );
    assert_eq!(
        foto(&c, &b, TABELAS_DA_ORG),
        banco_b_antes,
        "banco da outra org intacto"
    );
}

#[test]
fn isolamento_entre_orgs_com_varredura_geral() {
    let cenario = Cenario::novo();
    let c = cenario.banco();
    let ok = semear(&c, "ok", "{}", 40);
    let ruim = semear(&c, "ruim", r#"{"retencao":{"dias":-5}}"#, 40);
    let antes_ruim = foto(&c, &ruim, TABELAS_DA_ORG);

    let relatorio =
        executar_todas(&cenario.armazem(), &opcoes(30, false)).expect("varredura geral");

    let de_ok = relatorio
        .orgs
        .iter()
        .find(|r| r.org_id == ok)
        .expect("org ok");
    let de_ruim = relatorio
        .orgs
        .iter()
        .find(|r| r.org_id == ruim)
        .expect("org ruim");
    assert_eq!(de_ok.situacao, Situacao::Varrida);
    assert_eq!(banco_da(de_ok).mensagens, 3);
    assert_eq!(de_ruim.situacao, Situacao::Erro);
    assert!(
        de_ruim
            .motivo
            .as_deref()
            .unwrap_or("")
            .contains("Nada foi apagado"),
        "{:?}",
        de_ruim.motivo
    );
    assert_eq!(
        foto(&c, &ruim, TABELAS_DA_ORG),
        antes_ruim,
        "org com config inválida não é tocada"
    );
    assert_eq!(contar(&c, "messages", &ok), 2, "a org boa foi varrida");
}

#[test]
fn id_de_org_inexistente_nao_apaga_nada() {
    let cenario = Cenario::novo();
    let c = cenario.banco();
    let org = semear(&c, "existe", "{}", 40);
    let antes = foto(&c, &org, TABELAS_DA_ORG);

    let erro = executar_org(&cenario.armazem(), OrgId::generate(), &opcoes(30, false))
        .expect_err("org inexistente tem de falhar");

    assert!(matches!(erro, ErroRetencao::OrgInexistente(_)), "{erro:?}");
    assert_eq!(foto(&c, &org, TABELAS_DA_ORG), antes);
}

#[test]
fn pendentes_nunca_sao_apagados() {
    let cenario = Cenario::novo();
    let c = cenario.banco();
    let org = semear(&c, "pendentes", "{}", 400);
    // Mais casos de pendência, todos antigos: nada disso pode sair.
    let ag = agente(&c, &org);
    let conv = conversa(&c, &org, &ag, AGORA - 400 * DIA);
    let t_rodando = turno(&c, &org, &conv, &ag, "running", AGORA - 400 * DIA, None);
    saida(
        &c,
        &org,
        &conv,
        &t_rodando,
        "sending",
        AGORA - 400 * DIA,
        None,
    );
    entrada(&c, &org, AGORA - 400 * DIA, None);
    let t_falha = turno(
        &c,
        &org,
        &conv,
        &ag,
        "failed",
        AGORA - 400 * DIA,
        Some(AGORA - 400 * DIA),
    );
    saida(&c, &org, &conv, &t_falha, "failed", AGORA - 400 * DIA, None);
    let nao_enviadas = |c: &Connection| -> i64 {
        c.query_row(
            "SELECT COUNT(*) FROM outbox WHERE org_id = ?1 AND state <> 'sent'",
            params![org],
            |linha| linha.get(0),
        )
        .expect("saídas não enviadas")
    };
    let sem_processar = |c: &Connection| -> i64 {
        c.query_row(
            "SELECT COUNT(*) FROM inbox WHERE org_id = ?1 AND processed_at IS NULL",
            params![org],
            |linha| linha.get(0),
        )
        .expect("entradas não processadas")
    };
    let rodando = |c: &Connection| -> i64 {
        c.query_row(
            "SELECT COUNT(*) FROM turns WHERE org_id = ?1 AND state = 'running'",
            params![org],
            |linha| linha.get(0),
        )
        .expect("turnos rodando")
    };
    let antes = (nao_enviadas(&c), sem_processar(&c), rodando(&c));
    assert_eq!(antes, (4, 2, 2), "cenário de partida");

    executar_org(&cenario.armazem(), org_id(&org), &opcoes(1, false)).expect("varredura");

    let depois = (nao_enviadas(&c), sem_processar(&c), rodando(&c));
    assert_eq!(
        depois, antes,
        "saídas pendentes, entradas e turnos rodando continuam"
    );
}

#[test]
fn auditoria_bate_com_o_que_foi_apagado() {
    let cenario = Cenario::novo();
    let c = cenario.banco();
    let org = semear(&c, "auditada", "{}", 40);
    let pasta = cenario.pasta_da_org(&org);
    gravar_arquivo(&pasta, &sha(1), 40 * DIA);

    let relatorio =
        executar_org(&cenario.armazem(), org_id(&org), &opcoes(30, false)).expect("varredura");
    let banco = banco_da(&relatorio);

    let auditoria = auditorias(&c, &org);
    assert_eq!(auditoria.len(), 1, "uma linha por org varrida");
    let detalhe = &auditoria[0];
    assert_eq!(detalhe["simulacao"], false);
    assert_eq!(detalhe["mensagens"], banco.mensagens);
    assert_eq!(detalhe["anexos"], banco.anexos);
    assert_eq!(detalhe["turnos"], banco.turnos);
    assert_eq!(detalhe["inbox"], banco.inbox);
    assert_eq!(detalhe["outbox"], banco.outbox);
    assert_eq!(detalhe["conversas"], banco.conversas);
    assert_eq!(detalhe["arquivos_apagados"], relatorio.arquivos_apagados);
    assert_eq!(
        detalhe["arquivos_sem_dono"],
        relatorio.arquivos_sem_dono.len()
    );
}

// ───────────── O que nunca sai: Fichas, execuções e fluxos (FX-34) ─────────────

/// Tabelas do módulo de Fluxos e Fichas: nenhuma delas pode perder linha na varredura.
const TABELAS_DO_MODULO: &[&str] = &[
    "fluxos",
    "fluxo_revisoes",
    "agente_fluxos",
    "fluxo_execucoes",
    "fluxo_execucao_etapas",
    "ficha_eventos",
    "fluxo_efeitos",
];

/// A varredura apaga a conversa vazia e antiga, mas **nada** do atendimento registrado: a Ficha,
/// a execução, as etapas, os eventos e os efeitos ficam, e a Ficha perde só o vínculo com a
/// conversa (`conversation_id` nulo). Vale também para a simulação, que não muda nada.
#[test]
fn ficha_execucao_etapa_evento_e_efeito_sobrevivem_a_varredura() {
    let cenario = Cenario::novo();
    let c = cenario.banco();
    let org_texto = org(&c, "modulo-fluxos", "{}");
    let org = org_id(&org_texto);
    let agente: AgentId = agente(&c, &org_texto).parse().expect("id do agente");

    // A conversa nasce pelo repositório (que cria a Ficha junto) e é envelhecida: é a conversa
    // vazia e antiga que a varredura apaga.
    let conversa = cenario
        .store
        .conversations()
        .create(
            org,
            NewConversation {
                agent_id: agente,
                integration_id: None,
                canal: "cli".to_owned(),
                peer_externo: Some("cliente-ficticio".to_owned()),
                target_department: None,
            },
        )
        .expect("conversa com Ficha");
    c.execute(
        "UPDATE conversations SET created_at = ?1, updated_at = ?1 WHERE org_id = ?2 AND id = ?3",
        params![AGORA - 400 * DIA, org_texto, conversa.id.to_string()],
    )
    .expect("envelhece a conversa");

    // Fluxo publicado (revisão 1), liberado ao agente e com execução aberta na Ficha.
    let fluxo = cenario
        .store
        .flows()
        .create(
            org,
            NewFlow {
                slug: "agendar_visita".to_owned(),
                name: "Agendar visita".to_owned(),
                description: String::new(),
                rascunho_fonte: "fluxo()".to_owned(),
            },
        )
        .expect("fluxo");
    let revisao = cenario
        .store
        .revisions()
        .publish_revision(
            org,
            fluxo.id,
            RevisionPublication {
                expected: fluxo.version,
                fonte: "fluxo()".to_owned(),
                definicao_json: "{}".to_owned(),
                schema_versao: Some(1),
                resolucoes_json: "{}".to_owned(),
                ator_tipo: "cli".to_owned(),
                actor_id: None,
            },
        )
        .expect("revisão publicada");
    cenario
        .store
        .flows()
        .release(org, agente, fluxo.id)
        .expect("libera o fluxo ao agente");
    let ficha = cenario
        .store
        .cases()
        .by_conversation(org, conversa.id)
        .expect("consulta")
        .expect("a conversa tem Ficha");
    cenario
        .store
        .executions()
        .start_execution(
            org,
            ExecutionStart {
                case: ficha.id,
                flow: fluxo.id,
                revision: revisao.id,
                actor: EventActor::Agent,
                ator_ref: Some("agente-ficticio"),
                jev_decision: None,
                initial_state: ExecutionState {
                    status: ExecutionStatus::Active,
                    no_atual: "coleta".to_owned(),
                    dados_json: "{}".to_owned(),
                    acting_actors: Vec::new(),
                    result: None,
                    reason: None,
                },
                inbound_effects: vec![TransitionEffect {
                    key: "entrada/0".to_owned(),
                    tipo: "mensagem_ao_cliente".to_owned(),
                    payload_json: "{\"texto\":\"olá\"}".to_owned(),
                }],
                chave_idempotencia: None,
            },
        )
        .expect("abre a execução");

    let antes = foto(&c, &org_texto, TABELAS_DO_MODULO);
    assert_eq!(antes["fluxo_execucoes"].len(), 1);
    assert_eq!(antes["fluxo_execucao_etapas"].len(), 1);
    assert_eq!(antes["fluxo_efeitos"].len(), 1);
    assert!(antes["ficha_eventos"].len() >= 2, "aberta + fluxo iniciado");

    // Simulação: conta a conversa que sairia e não muda nada.
    let simulacao = executar_org(&cenario.armazem(), org, &opcoes(30, true)).expect("simulação");
    assert_eq!(simulacao.situacao, Situacao::Varrida);
    assert_eq!(banco_da(&simulacao).conversas, 1, "a conversa vazia sairia");
    assert_eq!(
        foto(&c, &org_texto, TABELAS_DO_MODULO),
        antes,
        "a simulação não apaga nada do módulo"
    );
    assert_eq!(
        contar(&c, "conversations", &org_texto),
        1,
        "a simulação não apaga a conversa"
    );

    // Varredura de verdade: a conversa sai, o atendimento registrado fica.
    let real = executar_org(&cenario.armazem(), org, &opcoes(30, false)).expect("varredura");
    assert_eq!(banco_da(&real).conversas, 1);
    assert_eq!(contar(&c, "conversations", &org_texto), 0);
    assert_eq!(
        foto(&c, &org_texto, TABELAS_DO_MODULO),
        antes,
        "Ficha, execução, etapa, evento e efeito não podem perder linha"
    );

    // A Ficha continua aberta e perde só o vínculo com a conversa apagada.
    assert_eq!(contar(&c, "fichas", &org_texto), 1);
    let (situacao, conversa_da_ficha): (String, Option<String>) = c
        .query_row(
            "SELECT status, conversation_id FROM fichas WHERE org_id = ?1",
            params![org_texto],
            |linha| Ok((linha.get(0)?, linha.get(1)?)),
        )
        .expect("Ficha depois da varredura");
    assert_eq!(situacao, "open", "a Ficha do atendimento não fecha sozinha");
    assert_eq!(
        conversa_da_ficha, None,
        "conversa apagada deixa a Ficha com o vínculo nulo"
    );
    assert_eq!(contar(&c, "fluxo_execucoes", &org_texto), 1);
    assert_eq!(contar(&c, "fluxo_execucao_etapas", &org_texto), 1);
    assert_eq!(contar(&c, "fluxo_efeitos", &org_texto), 1);
    assert_eq!(contar(&c, "fluxo_revisoes", &org_texto), 1);
    assert_eq!(contar(&c, "fluxos", &org_texto), 1);
}
