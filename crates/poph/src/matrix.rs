//! `poph matrix`: gera e confere o registro do Application Service do módulo Matrix (MX-21).
//!
//! O Synapse só conhece um Application Service pelo arquivo de registro. Esse arquivo é **gerado**,
//! nunca versionado (PRD §6.2): ele carrega os dois tokens, e token não entra em git.
//!
//! - `poph matrix registro --saida <arquivo> [--url <base>]` escreve o registro com permissão 0600,
//!   recusa sobrescrever sem `--forcar` e nunca imprime token (nem em erro).
//! - `poph matrix registro --verificar <arquivo>` relê o arquivo e compara com a seção `[matrix]`
//!   atual (prefixo, url, `sender_localpart`, namespaces, `rate_limited`) e com os tokens do
//!   ambiente; divergência é erro que **nomeia o campo**, sem repetir valor.
//!
//! O YAML é escrito à mão: o workspace não tem crate de YAML, e o formato é o mesmo do modelo da
//! MX-01 (`tests/matrix/registro-teste.yaml.template`), pequeno e estável. A leitura do `--verificar`
//! é um leitor igualmente simples do mesmo formato — não é um parser de YAML genérico de propósito.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use poph_matrix::{SegredosMatrix, VAR_AS_TOKEN, VAR_HS_TOKEN};
use poph_types::MatrixConfig;

use crate::config;
use crate::matrix_admin::{self, EspacoArgs, SaidaArgs};

/// Id do Application Service quando `--id` não é dado.
pub const ID_PADRAO: &str = "poph-matrix";

/// Variável de ambiente que serve de padrão para a `url` do registro (`--url` vence).
///
/// A seção `[matrix]` guarda o endereço do **Synapse** (`homeserver_url`), não o do Harness: quem
/// diz por onde o Synapse alcança o Harness é esta variável ou `--url`.
pub const VAR_URL_AS: &str = "POPH_MATRIX_URL_AS";

/// Alias compartilhado com o registro do Python, não exclusivo (PRD §6.2).
pub const ALIAS_COMPARTILHADO: &str = "#dep_.*";

/// Subcomandos de `poph matrix`.
///
/// `registro` não precisa de org. Os outros quatro operam numa org (`--org`) direto no banco
/// (`--offline`), como os demais comandos administrativos; a regra fica em `matrix_admin.rs`.
#[derive(Debug, Subcommand)]
pub enum AcaoMatrix {
    /// Gera (`--saida`) ou confere (`--verificar`) o registro do Application Service.
    Registro(RegistroArgs),
    /// Estado do módulo na org: ligado, corte do backfill, atraso, falhas (`--offline --org`).
    Estado(SaidaArgs),
    /// Liga o módulo na org; grava o corte do backfill na primeira vez (`--offline --org`).
    Ligar(SaidaArgs),
    /// Desliga o módulo na org; preserva o corte e não apaga nada no Element (`--offline --org`).
    Desligar(SaidaArgs),
    /// Espaços por departamento: lista, cria ou liga (`--departamento`) e `desligar` (`--offline --org`).
    Espaco(EspacoArgs),
}

/// Opções de `poph matrix registro`.
#[derive(Debug, Args)]
pub struct RegistroArgs {
    /// Arquivo a gerar, com permissão 0600. Recusa sobrescrever sem `--forcar`.
    #[arg(long, value_name = "ARQUIVO")]
    pub saida: Option<PathBuf>,

    /// Arquivo existente a conferir contra a seção `[matrix]` e os tokens do ambiente.
    #[arg(long, value_name = "ARQUIVO")]
    pub verificar: Option<PathBuf>,

    /// Endereço por onde o Synapse alcança o Harness (padrão: variável `POPH_MATRIX_URL_AS`).
    #[arg(long, value_name = "BASE")]
    pub url: Option<String>,

    /// Id do Application Service no registro (muda por instância na fase 3).
    #[arg(long, value_name = "ID", default_value = ID_PADRAO)]
    pub id: String,

    /// Sobrescreve o arquivo existente.
    #[arg(long)]
    pub forcar: bool,
}

/// Executa `poph matrix …`.
pub fn executar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    offline: bool,
    org: Option<&str>,
    acao: AcaoMatrix,
) -> ExitCode {
    match acao {
        AcaoMatrix::Registro(args) => executar_registro(flag, ambiente, args),
        AcaoMatrix::Estado(args) => matrix_admin::estado(flag, ambiente, offline, org, &args),
        AcaoMatrix::Ligar(args) => {
            matrix_admin::ligar_ou_desligar(flag, ambiente, offline, org, true, &args)
        }
        AcaoMatrix::Desligar(args) => {
            matrix_admin::ligar_ou_desligar(flag, ambiente, offline, org, false, &args)
        }
        AcaoMatrix::Espaco(args) => matrix_admin::espaco(flag, ambiente, offline, org, &args),
    }
}

/// Executa `poph matrix registro`.
fn executar_registro(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    args: RegistroArgs,
) -> ExitCode {
    match (args.saida.as_deref(), args.verificar.as_deref()) {
        (Some(_), Some(_)) => erro("use --saida (gerar) ou --verificar (conferir), não os dois"),
        (None, None) => {
            erro("informe --saida <arquivo> para gerar ou --verificar <arquivo> para conferir")
        }
        (Some(destino), None) => gerar(flag, ambiente, &args, destino),
        (None, Some(arquivo)) => conferir(flag, ambiente, &args, arquivo),
    }
}

/// `poph matrix registro --saida`: monta e grava o registro.
fn gerar(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    args: &RegistroArgs,
    destino: &Path,
) -> ExitCode {
    let (config, segredos) = match ler_ambiente(flag, ambiente) {
        Ok(par) => par,
        Err(mensagem) => return erro(&mensagem),
    };
    let url = match url_do_registro(args, ambiente, &config) {
        Ok(url) => url,
        Err(mensagem) => return erro(&mensagem),
    };

    let texto = yaml(&args.id, &url, &config.prefix, &segredos);

    if let Err(mensagem) = gravar(destino, &texto, args.forcar) {
        return erro(&mensagem);
    }

    println!(
        "registro do Application Service gravado: {}",
        destino.display()
    );
    println!("  id:               {}", args.id);
    println!("  url:              {url}");
    println!("  sender_localpart: {}", sender_localpart(&config.prefix));
    println!(
        "  namespaces:       @{}_.* e #{}_.* (exclusivos), {ALIAS_COMPARTILHADO} (compartilhado)",
        config.prefix, config.prefix
    );
    println!("  rate_limited:     false");
    println!("  as_token:         [oculto]");
    println!("  hs_token:         [oculto]");
    println!("  permissão:        0600");
    println!();
    println!(
        "O Synapse só lê registros na subida: reinicie o Synapse deste ambiente para carregá-lo."
    );
    ExitCode::SUCCESS
}

/// `poph matrix registro --verificar`: relê o arquivo e aponta cada divergência.
fn conferir(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
    args: &RegistroArgs,
    arquivo: &Path,
) -> ExitCode {
    let (config, segredos) = match ler_ambiente(flag, ambiente) {
        Ok(par) => par,
        Err(mensagem) => return erro(&mensagem),
    };
    let url = match url_do_registro(args, ambiente, &config) {
        Ok(url) => url,
        Err(mensagem) => return erro(&mensagem),
    };
    let texto = match fs::read_to_string(arquivo) {
        Ok(texto) => texto,
        Err(motivo) => {
            return erro(&format!(
                "não consegui ler o registro {}: {motivo}",
                arquivo.display()
            ));
        }
    };
    let lido = match ler(&texto) {
        Ok(lido) => lido,
        Err(mensagem) => {
            return erro(&format!("{}: {mensagem}", arquivo.display()));
        }
    };

    let esperado = Esperado {
        url: &url,
        prefixo: &config.prefix,
        segredos: &segredos,
    };
    let divergencias = comparar(&lido, &esperado);
    if !divergencias.is_empty() {
        eprintln!(
            "{}: registro divergente da configuração atual:",
            arquivo.display()
        );
        for divergencia in &divergencias {
            eprintln!("  {divergencia}");
        }
        eprintln!(
            "\nGere de novo com `poph matrix registro --saida <arquivo> --forcar` ou ajuste a seção [matrix]."
        );
        return ExitCode::from(1);
    }

    println!("registro em dia: {}", arquivo.display());
    println!("  url:              {url}");
    println!("  sender_localpart: {}", sender_localpart(&config.prefix));
    println!("  rate_limited:     false");
    println!("  tokens:           conferem com o ambiente ([oculto])");
    ExitCode::SUCCESS
}

/// Configuração efetiva e os tokens: o que o registro gerado precisa ter.
fn ler_ambiente(
    flag: Option<PathBuf>,
    ambiente: &config::Ambiente,
) -> Result<(MatrixConfig, SegredosMatrix), String> {
    let pasta = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let arquivo = config::resolver_arquivo(flag.as_deref(), ambiente, &pasta);
    let config = config::carregar(arquivo.as_deref(), ambiente).map_err(|erro| erro.to_string())?;
    let Some(secao) = config.matrix.as_ref() else {
        return Err(
            "seção [matrix] ausente na configuração: declare-a no poph.toml (ou por --config) antes de \
             gerar o registro"
                .to_owned(),
        );
    };
    // O registro descreve a instância mesmo quando o módulo ainda está desligado: validamos a seção
    // como se estivesse ligada, para não gerar registro com prefixo ou url inválidos.
    let mut efetiva = secao.clone();
    efetiva.enabled = true;
    efetiva.validar().map_err(|erro| erro.to_string())?;
    let segredos =
        SegredosMatrix::de(|nome| ambiente.get(nome).cloned()).map_err(|erro| erro.to_string())?;
    Ok((efetiva, segredos))
}

/// `--url` vence; sem ela, `POPH_MATRIX_URL_AS`; sem as duas, erro que explica o que é.
fn url_do_registro(
    args: &RegistroArgs,
    ambiente: &config::Ambiente,
    _config: &MatrixConfig,
) -> Result<String, String> {
    let bruta = args
        .url
        .clone()
        .or_else(|| ambiente.get(VAR_URL_AS).cloned())
        .unwrap_or_default();
    let url = bruta.trim().trim_end_matches('/');
    if url.is_empty() {
        return Err(format!(
            "informe a url do Harness com --url (ou defina {VAR_URL_AS}): é o endereço por onde o \
             Synapse alcança o Harness, ex.: http://192.168.0.123:8080"
        ));
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(
            "campo url: use http:// ou https:// no começo (endereço por onde o Synapse alcança o \
             Harness)"
                .to_owned(),
        );
    }
    Ok(url.to_owned())
}

/// `sender_localpart` do registro: `<prefixo>_as_bot`, como no modelo da MX-01.
pub fn sender_localpart(prefixo: &str) -> String {
    format!("{prefixo}_as_bot")
}

/// Monta o YAML do registro. Os tokens saem só daqui para o arquivo (nunca para a saída padrão).
pub fn yaml(id: &str, url: &str, prefixo: &str, segredos: &SegredosMatrix) -> String {
    let mut texto = String::new();
    texto.push_str(
        "# ═══════════════════════════════════════════════════════════════════════════\n\
         # Matrix Application Service do POP Harness — ARQUIVO GERADO, NUNCA VERSIONADO.\n\
         #\n\
         # Gerado por `poph matrix registro --saida <arquivo>` (MX-21). Contém os dois tokens do\n\
         # Application Service: guarde com permissão 0600 e não copie para o repositório.\n\
         # O Synapse só lê registros na subida: reinicie o Synapse para carregar alterações.\n\
         # ═══════════════════════════════════════════════════════════════════════════\n",
    );
    texto.push_str(&format!("id: {}\n", escalar(id)));
    texto.push_str(&format!("url: {}\n", escalar(url)));
    texto.push_str(&format!(
        "as_token: {}\n",
        escalar(segredos.as_token.expose())
    ));
    texto.push_str(&format!(
        "hs_token: {}\n",
        escalar(segredos.hs_token.expose())
    ));
    // MX-02 (item 7): sem `rate_limited: false` o Synapse limita o bot em rajada.
    texto.push_str("rate_limited: false\n");
    texto.push_str(&format!(
        "sender_localpart: {}\n",
        escalar(&sender_localpart(prefixo))
    ));
    texto.push_str("namespaces:\n");
    texto.push_str("  users:\n");
    texto.push_str("    - exclusive: true\n");
    texto.push_str(&format!(
        "      regex: {}\n",
        escalar(&format!("@{prefixo}_.*"))
    ));
    texto.push_str("  aliases:\n");
    texto.push_str("    - exclusive: true\n");
    texto.push_str(&format!(
        "      regex: {}\n",
        escalar(&format!("#{prefixo}_.*"))
    ));
    texto.push_str("    - exclusive: false\n");
    texto.push_str(&format!("      regex: {}\n", escalar(ALIAS_COMPARTILHADO)));
    texto.push_str("  rooms: []\n");
    texto
}

/// Valor escalar em YAML entre aspas duplas, com escape do que o formato exige.
///
/// Tokens gerados são hexadecimais, mas a url e o id vêm de fora: nunca confie no formato.
fn escalar(valor: &str) -> String {
    let mut saida = String::with_capacity(valor.len() + 2);
    saida.push('"');
    for caractere in valor.chars() {
        match caractere {
            '"' => saida.push_str("\\\""),
            '\\' => saida.push_str("\\\\"),
            '\n' => saida.push_str("\\n"),
            '\r' => saida.push_str("\\r"),
            '\t' => saida.push_str("\\t"),
            outro => saida.push(outro),
        }
    }
    saida.push('"');
    saida
}

/// Grava o arquivo com permissão 0600, sem sobrescrever sem `--forcar`.
fn gravar(destino: &Path, texto: &str, forcar: bool) -> Result<(), String> {
    if destino.exists() && !forcar {
        return Err(format!(
            "o arquivo {} já existe: use --forcar para sobrescrever",
            destino.display()
        ));
    }
    let mut arquivo = abrir_0600(destino, forcar).map_err(|erro| {
        format!(
            "não consegui criar o registro {}: {erro}",
            destino.display()
        )
    })?;
    arquivo.write_all(texto.as_bytes()).map_err(|erro| {
        format!(
            "não consegui escrever o registro {}: {erro}",
            destino.display()
        )
    })?;
    arquivo.flush().map_err(|erro| {
        format!(
            "não consegui terminar de escrever o registro {}: {erro}",
            destino.display()
        )
    })?;
    Ok(())
}

#[cfg(unix)]
fn abrir_0600(destino: &Path, forcar: bool) -> std::io::Result<fs::File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut opcoes = fs::OpenOptions::new();
    opcoes.write(true).mode(0o600);
    if forcar {
        opcoes.create(true).truncate(true);
    } else {
        opcoes.create_new(true);
    }
    let arquivo = opcoes.open(destino)?;
    // Arquivo que já existia com permissão larga também fica 0600 depois do --forcar.
    arquivo.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(arquivo)
}

#[cfg(not(unix))]
fn abrir_0600(destino: &Path, forcar: bool) -> std::io::Result<fs::File> {
    let mut opcoes = fs::OpenOptions::new();
    opcoes.write(true);
    if forcar {
        opcoes.create(true).truncate(true);
    } else {
        opcoes.create_new(true);
    }
    opcoes.open(destino)
}

// ─────────────────────────── leitura do registro (--verificar) ───────────────────────────

/// Um item de namespace de lista: `exclusive` e o `regex`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemNamespace {
    /// `exclusive: true` = só este Application Service usa o namespace.
    pub exclusivo: bool,
    /// Expressão regular do namespace.
    pub regex: String,
}

/// O registro relido: os campos escalares, os namespaces de usuários e os de aliases.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RegistroLido {
    /// Campos escalares do topo, sem as aspas (`id`, `url`, `as_token`, …).
    pub escalares: BTreeMap<String, String>,
    /// `namespaces.users`, na ordem do arquivo.
    pub usuarios: Vec<ItemNamespace>,
    /// `namespaces.aliases`, na ordem do arquivo.
    pub aliases: Vec<ItemNamespace>,
}

/// Lê o formato que este comando gera (e o do modelo da MX-01).
///
/// Não é um leitor de YAML genérico: aceita exatamente escalares no topo, `namespaces.users`,
/// `namespaces.aliases` e `namespaces.rooms` (lista vazia ou simples, ignorada). Qualquer outra
/// coisa é erro que nomeia a linha, para não conferir um arquivo que não entendeu.
pub fn ler(texto: &str) -> Result<RegistroLido, String> {
    #[derive(Default)]
    struct Estado {
        dentro_de_namespaces: bool,
        lista: Option<&'static str>,
        item: Option<ItemNamespace>,
        exclusive_lido: bool,
    }

    fn fechar(lido: &mut RegistroLido, estado: &mut Estado) -> Result<(), String> {
        let Some(item) = estado.item.take() else {
            return Ok(());
        };
        if !estado.exclusive_lido || item.regex.is_empty() {
            return Err(format!(
                "namespaces.{}: item sem `exclusive` e `regex`",
                estado.lista.unwrap_or("?")
            ));
        }
        match estado.lista {
            Some("users") => lido.usuarios.push(item),
            Some("aliases") => lido.aliases.push(item),
            _ => {}
        }
        estado.exclusive_lido = false;
        Ok(())
    }

    let mut lido = RegistroLido::default();
    let mut estado = Estado::default();

    for (numero, bruta) in texto.lines().enumerate() {
        let linha = bruta.trim_end();
        let conteudo = linha.trim();
        if conteudo.is_empty() || conteudo.starts_with('#') {
            continue;
        }
        let recuo = linha.len() - linha.trim_start().len();
        let onde = format!("linha {}", numero + 1);

        if recuo == 0 {
            fechar(&mut lido, &mut estado)?;
            let Some((chave, valor)) = conteudo.split_once(':') else {
                return Err(format!(
                    "{onde}: esperava `chave: valor`, achei `{conteudo}`"
                ));
            };
            let chave = chave.trim();
            let valor = valor.trim();
            if chave == "namespaces" {
                if !valor.is_empty() {
                    return Err(format!("{onde}: `namespaces` não é escalar"));
                }
                estado.dentro_de_namespaces = true;
                estado.lista = None;
                continue;
            }
            if valor.is_empty() {
                return Err(format!("{onde}: campo `{chave}` sem valor"));
            }
            lido.escalares
                .insert(chave.to_owned(), sem_aspas(valor).to_owned());
            continue;
        }

        if !estado.dentro_de_namespaces {
            return Err(format!("{onde}: linha recuada fora de `namespaces`"));
        }

        if recuo == 2 {
            fechar(&mut lido, &mut estado)?;
            let Some((chave, _)) = conteudo.split_once(':') else {
                return Err(format!("{onde}: esperava `users:`, `aliases:` ou `rooms:`"));
            };
            estado.lista = match chave.trim() {
                "users" => Some("users"),
                "aliases" => Some("aliases"),
                "rooms" => None,
                outra => {
                    return Err(format!(
                        "{onde}: namespaces desconhecido `{outra}` (esperava users, aliases ou rooms)"
                    ));
                }
            };
            continue;
        }

        if estado.lista.is_none() {
            return Err(format!("{onde}: item em `namespaces.rooms` não esperado"));
        }

        if let Some(resto) = conteudo.strip_prefix('-') {
            fechar(&mut lido, &mut estado)?;
            let resto = resto.trim();
            if resto.is_empty() {
                // `-` sozinho: o item começa na linha seguinte (estilo bloco).
                estado.item = Some(ItemNamespace {
                    exclusivo: false,
                    regex: String::new(),
                });
                continue;
            }
            let Some((chave, valor)) = resto.split_once(':') else {
                return Err(format!("{onde}: esperava `- exclusive: true|false`"));
            };
            let chave = chave.trim();
            let valor = sem_aspas(valor.trim());
            let mut novo = ItemNamespace {
                exclusivo: false,
                regex: String::new(),
            };
            match chave {
                "exclusive" => {
                    novo.exclusivo = exclusividade(chave, valor, &onde)?;
                    estado.exclusive_lido = true;
                }
                "regex" => novo.regex = valor.to_owned(),
                outra => {
                    return Err(format!(
                        "{onde}: chave desconhecida no item: `{outra}` (esperava exclusive ou regex)"
                    ));
                }
            }
            estado.item = Some(novo);
            continue;
        }

        let Some((chave, valor)) = conteudo.split_once(':') else {
            return Err(format!("{onde}: esperava `regex: ...` dentro do item"));
        };
        match chave.trim() {
            "regex" => {
                let Some(item) = estado.item.as_mut() else {
                    return Err(format!("{onde}: `regex` sem item"));
                };
                item.regex = sem_aspas(valor.trim()).to_owned();
            }
            "exclusive" => {
                let exclusivo = exclusividade("exclusive", sem_aspas(valor.trim()), &onde)?;
                let Some(item) = estado.item.as_mut() else {
                    return Err(format!("{onde}: `exclusive` sem item"));
                };
                item.exclusivo = exclusivo;
                estado.exclusive_lido = true;
            }
            outra => return Err(format!("{onde}: chave desconhecida no item: `{outra}`")),
        }
    }
    fechar(&mut lido, &mut estado)?;
    if lido.escalares.is_empty() {
        return Err("arquivo sem nenhum campo do registro".to_owned());
    }
    Ok(lido)
}

fn exclusividade(chave: &str, valor: &str, onde: &str) -> Result<bool, String> {
    match valor {
        "true" => Ok(true),
        "false" => Ok(false),
        outro => Err(format!(
            "{onde}: `{chave}` esperava true ou false, achei `{outro}`"
        )),
    }
}

/// Tira as aspas duplas de um valor escalar, se houver, e desfaz os escapes que geramos.
fn sem_aspas(valor: &str) -> &str {
    let Some(sem_primeira) = valor.strip_prefix('"') else {
        return valor;
    };
    sem_primeira.strip_suffix('"').unwrap_or(valor)
}

// ─────────────────────────────── comparação com a configuração ───────────────────────────────

/// O que o registro deveria dizer, segundo a seção `[matrix]` e o ambiente.
struct Esperado<'a> {
    url: &'a str,
    prefixo: &'a str,
    segredos: &'a SegredosMatrix,
}

/// Lista de divergências, cada uma nomeando o campo. Vazia = registro em dia.
fn comparar(lido: &RegistroLido, esperado: &Esperado<'_>) -> Vec<String> {
    let mut divergencias = Vec::new();
    let mut comparar_campo =
        |campo: &str, encontrado: Option<&str>, devia_ser: &str| match encontrado {
            Some(encontrado) if encontrado == devia_ser => {}
            Some(encontrado) => divergencias.push(format!(
                "campo {campo}: esperado `{devia_ser}`, encontrado `{encontrado}`"
            )),
            None => divergencias.push(format!("campo {campo}: ausente (esperado `{devia_ser}`)")),
        };

    comparar_campo(
        "url",
        lido.escalares.get("url").map(String::as_str),
        esperado.url,
    );
    comparar_campo(
        "sender_localpart",
        lido.escalares.get("sender_localpart").map(String::as_str),
        &sender_localpart(esperado.prefixo),
    );
    comparar_campo(
        "rate_limited",
        lido.escalares.get("rate_limited").map(String::as_str),
        "false",
    );
    comparar_campo("as_token", presenca(lido, "as_token"), "presente");
    comparar_campo("hs_token", presenca(lido, "hs_token"), "presente");

    // Tokens: iguais aos do ambiente? O valor nunca sai daqui, só o nome do campo.
    for (campo, no_arquivo, no_ambiente) in [
        (
            "as_token",
            lido.escalares.get("as_token"),
            Some(esperado.segredos.as_token.expose().as_str()),
        ),
        (
            "hs_token",
            lido.escalares.get("hs_token"),
            Some(esperado.segredos.hs_token.expose().as_str()),
        ),
    ] {
        if let (Some(no_arquivo), Some(no_ambiente)) = (no_arquivo, no_ambiente)
            && no_arquivo != no_ambiente
        {
            let variavel = if campo == "as_token" {
                VAR_AS_TOKEN
            } else {
                VAR_HS_TOKEN
            };
            divergencias.push(format!(
                "campo {campo}: o valor do arquivo difere de {variavel} no ambiente (gere de novo)"
            ));
        }
    }

    let usuario_esperado = ItemNamespace {
        exclusivo: true,
        regex: format!("@{}_.*", esperado.prefixo),
    };
    if !lido.usuarios.contains(&usuario_esperado) {
        divergencias.push(format!(
            "campo namespaces.users: falta o namespace exclusivo `{}`",
            usuario_esperado.regex
        ));
    }
    let alias_esperado = ItemNamespace {
        exclusivo: true,
        regex: format!("#{}_.*", esperado.prefixo),
    };
    if !lido.aliases.contains(&alias_esperado) {
        divergencias.push(format!(
            "campo namespaces.aliases: falta o namespace exclusivo `{}`",
            alias_esperado.regex
        ));
    }
    let alias_compartilhado = ItemNamespace {
        exclusivo: false,
        regex: ALIAS_COMPARTILHADO.to_owned(),
    };
    if !lido.aliases.contains(&alias_compartilhado) {
        divergencias.push(format!(
            "campo namespaces.aliases: falta o alias compartilhado `{ALIAS_COMPARTILHADO}` com \
             `exclusive: false`"
        ));
    }
    divergencias
}

fn presenca<'a>(lido: &'a RegistroLido, campo: &str) -> Option<&'a str> {
    lido.escalares
        .get(campo)
        .map(String::as_str)
        .filter(|valor| !valor.trim().is_empty())
        .map(|_| "presente")
}

/// Imprime o erro (sem token) e devolve o código de saída.
fn erro(mensagem: &str) -> ExitCode {
    eprintln!("erro: {mensagem}");
    ExitCode::from(1)
}

#[cfg(test)]
mod testes {
    use super::*;
    use poph_types::Sensitive;

    const CANARIO_AS: &str = "CANARIO-MX21-AS-nao-pode-vazar";
    const CANARIO_HS: &str = "CANARIO-MX21-HS-nao-pode-vazar";

    fn segredos() -> SegredosMatrix {
        SegredosMatrix {
            as_token: Sensitive::new(CANARIO_AS.to_owned()),
            hs_token: Sensitive::new(CANARIO_HS.to_owned()),
        }
    }

    #[test]
    fn yaml_gerado_relido_bate_com_a_configuracao() {
        let texto = yaml(
            "poph-matrix",
            "http://192.168.0.123:8080",
            "ph",
            &segredos(),
        );
        let lido = ler(&texto).expect("o próprio yaml gerado tem que ser legível");
        assert_eq!(lido.escalares["id"], "poph-matrix");
        assert_eq!(lido.escalares["url"], "http://192.168.0.123:8080");
        assert_eq!(lido.escalares["as_token"], CANARIO_AS);
        assert_eq!(lido.escalares["sender_localpart"], "ph_as_bot");
        assert_eq!(lido.escalares["rate_limited"], "false");
        assert_eq!(
            lido.usuarios,
            vec![ItemNamespace {
                exclusivo: true,
                regex: "@ph_.*".to_owned()
            }]
        );
        assert_eq!(
            lido.aliases,
            vec![
                ItemNamespace {
                    exclusivo: true,
                    regex: "#ph_.*".to_owned()
                },
                ItemNamespace {
                    exclusivo: false,
                    regex: "#dep_.*".to_owned()
                }
            ]
        );

        let esperado = Esperado {
            url: "http://192.168.0.123:8080",
            prefixo: "ph",
            segredos: &segredos(),
        };
        assert!(comparar(&lido, &esperado).is_empty());
    }

    #[test]
    fn prefixo_trocado_aponta_os_namespaces_e_o_sender() {
        let texto = yaml("poph-matrix", "http://h:1", "wa", &segredos());
        let lido = ler(&texto).expect("legível");
        let esperado = Esperado {
            url: "http://h:1",
            prefixo: "ph",
            segredos: &segredos(),
        };
        let divergencias = comparar(&lido, &esperado);
        let texto_das_divergencias = divergencias.join("\n");
        assert!(texto_das_divergencias.contains("campo sender_localpart"));
        assert!(texto_das_divergencias.contains("campo namespaces.users"));
        assert!(texto_das_divergencias.contains("campo namespaces.aliases"));
    }

    #[test]
    fn valor_canario_nao_aparece_nas_divergencias() {
        let texto = yaml("poph-matrix", "http://outro:2", "ph", &segredos());
        let lido = ler(&texto).expect("legível");
        let outros = SegredosMatrix {
            as_token: Sensitive::new("as-token-diferente".to_owned()),
            hs_token: Sensitive::new("hs-token-diferente".to_owned()),
        };
        let esperado = Esperado {
            url: "http://esperado:1",
            prefixo: "ph",
            segredos: &outros,
        };
        let divergencias = comparar(&lido, &esperado).join("\n");
        assert!(divergencias.contains("campo url"));
        assert!(divergencias.contains(VAR_AS_TOKEN));
        assert!(divergencias.contains(VAR_HS_TOKEN));
        for canario in [
            CANARIO_AS,
            CANARIO_HS,
            "as-token-diferente",
            "hs-token-diferente",
        ] {
            assert!(
                !divergencias.contains(canario),
                "o valor do token não pode aparecer: {divergencias}"
            );
        }
    }

    #[test]
    fn leitor_recusa_formato_que_nao_entende() {
        for (texto, trecho) in [
            ("id: x\nsem dois pontos\n", "linha 2"),
            (
                "id: x\nnamespaces:\n  users:\n    - regex: \"@ph_.*\"\n",
                "sem `exclusive`",
            ),
            (
                "id: x\nnamespaces:\n  outro: []\n",
                "namespaces desconhecido",
            ),
            (
                "id: x\nnamespaces:\n  users:\n    - exclusive: talvez\n",
                "esperava true ou false",
            ),
        ] {
            let erro = ler(texto).expect_err("deveria recusar");
            assert!(erro.contains(trecho), "mensagem: {erro}");
        }
    }

    #[test]
    fn leitor_aceita_o_modelo_da_mx01_com_comentarios() {
        let texto = r##"# comentário
id: poph-matrix-dev
url: "http://192.168.0.123:18010/matrix"
as_token: "abc"
hs_token: "def"
rate_limited: false
sender_localpart: ph_as_bot
namespaces:
  users:
    - exclusive: true
      regex: "@ph_.*"
  aliases:
    - exclusive: true
      regex: "#ph_.*"
    - exclusive: false
      regex: "#dep_.*"
  rooms: []
"##;
        let lido = ler(texto).expect("modelo da MX-01 tem que ser legível");
        assert_eq!(lido.usuarios.len(), 1);
        assert_eq!(lido.aliases.len(), 2);
        assert_eq!(lido.escalares["url"], "http://192.168.0.123:18010/matrix");
    }

    #[test]
    fn escalar_escapa_aspas_e_barras() {
        assert_eq!(escalar("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }
}
