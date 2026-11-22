//! Compositor das instruções do agente.
//!
//! Porte fiel de `core/agent/instructions_composer.py` do POP Atende (HR-03 §9.2): a mesma entrada
//! produz **o mesmo texto, byte a byte** — a mesma ordem de blocos, os mesmos separadores e as
//! mesmas omissões. A paridade é provada contra a saída real do compositor do POP Atende em
//! `tests/compositor_paridade.rs`.
//!
//! O que ele **não** faz (de propósito):
//! - não emite seção de handoff (o POP Atende removeu; os motivos de transferência são decisão do
//!   Jev e o estado da transferência entra como bloco volátil do turno, não no prefixo estável);
//! - não emite `flow_steps` (sem efeito no motor, igual ao POP Atende);
//! - não acrescenta estilo fixo do sistema, fichas, regras de data nem contexto temporal: isso é
//!   do motor do turno (PRD §6.1), como bloco próprio depois do prefixo estável.
//!
//! Acentos não são usados nos rótulos de propósito: é o que casa byte a byte com o original.

use poph_types::StructuredInstructions;

/// Compõe o markdown das instruções a partir das instruções estruturadas.
///
/// Saída determinística (mesma entrada → mesmos bytes) e sem `\n` no fim, exatamente como o
/// compositor do POP Atende.
pub fn compor_instrucoes(dados: &StructuredInstructions) -> String {
    let mut secoes: Vec<String> = Vec::new();

    // 1. Identidade — só com nome ou empresa; o papel e a empresa entram no cabeçalho.
    if !dados.identity.name.is_empty() || !dados.identity.company.is_empty() {
        let mut partes = String::new();
        if !dados.identity.name.is_empty() {
            partes.push_str(&dados.identity.name);
        }
        if !dados.identity.role.is_empty() {
            partes.push_str(", ");
            partes.push_str(&dados.identity.role);
        }
        if !dados.identity.company.is_empty() {
            partes.push_str(" da ");
            partes.push_str(&dados.identity.company);
        }
        secoes.push(format!("# {partes}\n"));
        if !dados.identity.company_description.is_empty() {
            secoes.push(dados.identity.company_description.clone());
        }
    }

    // 2. Princípios inegociáveis, numerados, cada um com a frase de contingência quando houver.
    if !dados.principles.is_empty() {
        secoes.push("\n## Principios inegociaveis\n".to_owned());
        for (indice, principio) in dados.principles.iter().enumerate() {
            let numero = indice + 1;
            let regra = &principio.rule;
            secoes.push(format!("{numero}. **{regra}**"));
            let contingencia = formatar_contingencia(&principio.fallback_action);
            if !contingencia.is_empty() {
                secoes.push(format!("   - {contingencia}"));
            }
        }
    }

    // 3. Escopo. (3b) Handoff não é emitido (ver nota do módulo.)
    if !dados.scope.does.is_empty() || !dados.scope.does_not.is_empty() {
        secoes.push("\n## Escopo\n".to_owned());
        if !dados.scope.does.is_empty() {
            secoes.push(format!("**O que voce faz:** {}\n", dados.scope.does));
        }
        if !dados.scope.does_not.is_empty() {
            secoes.push(format!(
                "**O que voce NAO faz:** {}\n",
                dados.scope.does_not
            ));
        }
    }

    // 4. Tom — sempre emitido, mesmo com o resto vazio.
    secoes.push("\n## Tom e Comunicacao\n".to_owned());
    secoes.push(format!(
        "- Idioma: {}",
        formatar_idioma(&dados.tone.language)
    ));
    secoes.push(format!("- Tom: {}", formatar_estilo(&dados.tone.style)));
    secoes.push(format!(
        "- Emojis: {}",
        formatar_emojis(&dados.tone.emoji_policy)
    ));
    secoes.push(format!(
        "- Formato: {}",
        formatar_formato(&dados.tone.format)
    ));
    if !dados.tone.extra_restrictions.is_empty() {
        secoes.push(format!("- {}", dados.tone.extra_restrictions));
    }

    // 5. Ferramentas — só as habilitadas.
    let habilitadas: Vec<_> = dados
        .tools
        .iter()
        .filter(|ferramenta| ferramenta.enabled)
        .collect();
    if !habilitadas.is_empty() {
        secoes.push("\n## Ferramentas Disponiveis\n".to_owned());
        for ferramenta in habilitadas {
            let descricao = if ferramenta.when_to_use.is_empty() {
                String::new()
            } else {
                format!(" — {}", ferramenta.when_to_use)
            };
            let nome = &ferramenta.name;
            secoes.push(format!("- `{nome}`{descricao}"));
        }
    }

    // 6. Instruções extras.
    if !dados.extra_instructions.is_empty() {
        secoes.push("\n## Instrucoes Adicionais\n".to_owned());
        secoes.push(dados.extra_instructions.clone());
    }

    secoes.join("\n")
}

/// Frase da ação de contingência. Ação desconhecida (ou vazia) não gera linha, como no original.
fn formatar_contingencia(acao: &str) -> &str {
    match acao {
        "say_dont_know" => "Se não souber, diga que não sabe e registre a dúvida.",
        "transfer" => "Deixe que o sistema decida o encaminhamento; você apenas acolha o cliente.",
        "escalate" => "Registre a necessidade de supervisão em comentário interno.",
        _ => "",
    }
}

/// Nome do idioma. Valor fora do mapa passa cru.
fn formatar_idioma(idioma: &str) -> &str {
    match idioma {
        "pt-BR" => "Portugues Brasileiro",
        "en" => "Ingles",
        "es" => "Espanhol",
        outra => outra,
    }
}

/// Nome do estilo. Valor fora do mapa passa cru.
fn formatar_estilo(estilo: &str) -> &str {
    match estilo {
        "professional" => "Profissional",
        "casual" => "Casual",
        "technical" => "Tecnico",
        "friendly" => "Amigavel",
        outro => outro,
    }
}

/// Política de emojis. Valor fora do mapa passa cru.
fn formatar_emojis(politica: &str) -> &str {
    match politica {
        "moderate" => "Com moderacao",
        "none" => "Sem emojis",
        "free" => "Livre",
        outra => outra,
    }
}

/// Formato de resposta. Valor fora do mapa passa cru.
fn formatar_formato(formato: &str) -> &str {
    match formato {
        "simple_lists" => "Listas simples (melhor para WhatsApp)",
        "free_text" => "Texto livre",
        "tables" => "Tabelas",
        outro => outro,
    }
}
