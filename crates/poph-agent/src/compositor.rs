use poph_types::StructuredInstructions;

pub fn compose_instructions(dados: &StructuredInstructions) -> String {
    let mut sections: Vec<String> = Vec::new();

    if !dados.identity.name.is_empty() || !dados.identity.company.is_empty() {
        let mut parts = String::new();
        if !dados.identity.name.is_empty() {
            parts.push_str(&dados.identity.name);
        }
        if !dados.identity.role.is_empty() {
            parts.push_str(", ");
            parts.push_str(&dados.identity.role);
        }
        if !dados.identity.company.is_empty() {
            parts.push_str(" da ");
            parts.push_str(&dados.identity.company);
        }
        sections.push(format!("# {parts}\n"));
        if !dados.identity.company_description.is_empty() {
            sections.push(dados.identity.company_description.clone());
        }
    }

    if !dados.principles.is_empty() {
        sections.push("\n## Principios inegociaveis\n".to_owned());
        for (index, principle) in dados.principles.iter().enumerate() {
            let numero = index + 1;
            let rule = &principle.rule;
            sections.push(format!("{numero}. **{rule}**"));
            let contingencia = format_contingencia(&principle.fallback_action);
            if !contingencia.is_empty() {
                sections.push(format!("   - {contingencia}"));
            }
        }
    }

    if !dados.scope.does.is_empty() || !dados.scope.does_not.is_empty() {
        sections.push("\n## Escopo\n".to_owned());
        if !dados.scope.does.is_empty() {
            sections.push(format!("**O que voce faz:** {}\n", dados.scope.does));
        }
        if !dados.scope.does_not.is_empty() {
            sections.push(format!(
                "**O que voce NAO faz:** {}\n",
                dados.scope.does_not
            ));
        }
    }

    sections.push("\n## Tom e Comunicacao\n".to_owned());
    sections.push(format!("- Idioma: {}", format_idioma(&dados.tone.language)));
    sections.push(format!("- Tom: {}", format_estilo(&dados.tone.style)));
    sections.push(format!(
        "- Emojis: {}",
        format_emojis(&dados.tone.emoji_policy)
    ));
    sections.push(format!("- Formato: {}", format_format(&dados.tone.format)));
    if !dados.tone.extra_restrictions.is_empty() {
        sections.push(format!("- {}", dados.tone.extra_restrictions));
    }

    let enabled: Vec<_> = dados.tools.iter().filter(|tool| tool.enabled).collect();
    if !enabled.is_empty() {
        sections.push("\n## Ferramentas Disponiveis\n".to_owned());
        for tool in enabled {
            let descricao = if tool.when_to_use.is_empty() {
                String::new()
            } else {
                format!(" — {}", tool.when_to_use)
            };
            let nome = &tool.name;
            sections.push(format!("- `{nome}`{descricao}"));
        }
    }

    if !dados.extra_instructions.is_empty() {
        sections.push("\n## Instrucoes Adicionais\n".to_owned());
        sections.push(dados.extra_instructions.clone());
    }

    sections.join("\n")
}

fn format_contingencia(acao: &str) -> &str {
    match acao {
        "say_dont_know" => "Se não souber, diga que não sabe e registre a dúvida.",
        "transfer" => "Deixe que o sistema decida o encaminhamento; você apenas acolha o cliente.",
        "escalate" => "Registre a necessidade de supervisão em comentário interno.",
        _ => "",
    }
}

fn format_idioma(idioma: &str) -> &str {
    match idioma {
        "pt-BR" => "Portugues Brasileiro",
        "en" => "Ingles",
        "es" => "Espanhol",
        other => other,
    }
}

fn format_estilo(estilo: &str) -> &str {
    match estilo {
        "professional" => "Profissional",
        "casual" => "Casual",
        "technical" => "Tecnico",
        "friendly" => "Amigavel",
        other => other,
    }
}

fn format_emojis(policy: &str) -> &str {
    match policy {
        "moderate" => "Com moderacao",
        "none" => "Sem emojis",
        "free" => "Livre",
        other => other,
    }
}

fn format_format(formato: &str) -> &str {
    match formato {
        "simple_lists" => "Listas simples (melhor para WhatsApp)",
        "free_text" => "Texto livre",
        "tables" => "Tabelas",
        other => other,
    }
}
