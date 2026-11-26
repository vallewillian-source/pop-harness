#!/usr/bin/env python3
import os
import sys
import shutil
import sqlite3
import subprocess
import json

def find_binary(root):
    override = os.environ.get("POPH_BIN")
    if override:
        if not os.path.isfile(override) or not os.access(override, os.X_OK):
            sys.stderr.write(f"POPH_BIN não é um executável: {override}\n")
            sys.exit(1)
        return override
    try:
        meta = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            cwd=root, capture_output=True, text=True, check=True,
        )
    except (OSError, subprocess.CalledProcessError):
        sys.stderr.write("cargo metadata falhou; não sei onde o Cargo guardou o poph\n")
        sys.exit(1)
    target = json.loads(meta.stdout)["target_directory"]
    for profile in ("debug", "release"):
        candidate = os.path.join(target, profile, "poph")
        if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
            return candidate
    sys.stderr.write(
        f"binário poph não encontrado em {target}/(debug|release); "
        "rode `cargo build --locked -p poph` ou defina POPH_BIN\n"
    )
    sys.exit(1)


def setup_demo_data(target_dir, port=17480):
    root = os.path.abspath(os.path.join(os.path.dirname(__file__), "../../.."))
    binary = find_binary(root)

    data_dir = os.path.join(target_dir, "data")
    if os.path.exists(target_dir):
        shutil.rmtree(target_dir)
    os.makedirs(data_dir, exist_ok=True)

    model_src = os.path.join(root, "data/modelos")
    if os.path.exists(model_src):
        shutil.copytree(model_src, os.path.join(data_dir, "modelos"))

    config_path = os.path.join(target_dir, "poph.toml")
    toml_content = f"""[servidor]
endereco = "127.0.0.1"
porta = {port}

[dados]
diretorio = "{data_dir}"

[admin]
tokens_globais = [
  {{ label = "demo", hash = "$argon2id$v=19$m=19456,t=2,p=1$4K6zKeDwBafMeJSyLp7lMg$XHfGn7qhg8i17nL8vYSmSoWhmuknuSC6MmzsgEVAAC4" }},
]
"""
    with open(config_path, "w", encoding="utf-8") as f:
        f.write(toml_content)

    env = os.environ.copy()
    env["POPH_CHAVE_CIFRA"] = "demo-cifra-123456789012345678901234"
    env["ANTIGRAVITY_API_KEY"] = "test"
    env["POPH_DADOS_DIRETORIO"] = data_dir
    env["POPH_CONFIG"] = config_path

    migrate_res = subprocess.run([binary, "migrate"], env=env, capture_output=True, text=True)
    if migrate_res.returncode != 0:
        sys.stderr.write(f"poph migrate failed:\n{migrate_res.stderr}\n")
        sys.exit(1)

    db_path = os.path.join(data_dir, "harness.db")
    conn = sqlite3.connect(db_path)
    c = conn.cursor()

    ts = 1795484000000

    c.execute("""
    INSERT INTO orgs (id, name, slug, status, config_json, version, created_at, updated_at)
    VALUES 
      ("01a20b31-0000-7000-8000-000000000001", "Acme Demonstração", "acme", "active", "{}", 1, ?, ?),
      ("01a20b31-0000-7000-8000-000000000002", "Beta Serviços", "beta", "active", "{}", 1, ?, ?)
    """, (ts, ts, ts, ts))

    c.execute("""
    INSERT INTO departments (id, org_id, name, description, created_at, updated_at)
    VALUES 
      ("01a20b31-0000-7000-8000-000000000010", "01a20b31-0000-7000-8000-000000000001", "Suporte", "Atendimento Geral", ?, ?)
    """, (ts, ts))

    c.execute("""
    INSERT INTO agents (
      id, org_id, name, slug, description, status, agent_type, category,
      default_department_id, human_handoff_enabled, human_handoff_department_id,
      provider, model, max_context_messages, max_context_tokens,
      grouping_window_ms, max_tool_rounds, max_tool_calls_per_message,
      rag_top_k, rag_min_score, jev_json, version, created_at, updated_at
    ) VALUES (
      "01a20b31-0000-7000-8000-000000000020", "01a20b31-0000-7000-8000-000000000001",
      "Bia Atendente", "bia-atendente", "Agente virtual de suporte da Acme Demonstração",
      "active", "custom", "generic", "01a20b31-0000-7000-8000-000000000010",
      0, NULL, "antigravity2api", "gemini-3.8-flash-tiered",
      30, 64000, 2000, 4, 10, 3, 0.0, "{}", 1, ?, ?
    )
    """, (ts, ts))

    inst_struct = {
        "identity": {
            "name": "Bia",
            "role": "Atendente Virtual",
            "company": "Acme Demonstração",
            "company_description": "Fornecedora de suprimentos industriais"
        },
        "scope": {
            "does": "Tirar dúvidas sobre produtos, entregas e horário de atendimento",
            "does_not": "Tratar de negociações financeiras complexas",
            "handoff": {
                "department_id": "01a20b31-0000-7000-8000-000000000010",
                "reasons": ["Solicitação explícita do cliente", "Reclamações críticas"]
            }
        },
        "tone": {
            "style": "professional",
            "format": "simple_lists",
            "language": "pt-BR",
            "emoji_policy": "moderate",
            "extra_restrictions": "Nunca prometer prazos que não constem na base"
        },
        "principles": [
            "Agir sempre com gentileza e brevidade.",
            "Consultar a base de conhecimento antes de responder."
        ],
        "flow_steps": [],
        "tools": [],
        "extra_instructions": ""
    }
    inst_md = """# Bia, Atendente Virtual da Acme Demonstração

## Identidade e Papel
Você é a Bia, atendente virtual da Acme Demonstração, responsável por orientar clientes com agilidade e cordialidade.

## Diretrizes de Tom e Comunicação
- Linguagem: Português do Brasil
- Estilo: Profissional e acolhedor
- Formatação: Mensagens curtas e listas simples ideais para leitura rápida no WhatsApp
- Emojis: Uso moderado

## Escopo
- Responder sobre horários de funcionamento, catálogo e prazos de entrega.
- Transferir para um atendente humano quando solicitado pelo cliente.
"""

    c.execute("""
    INSERT INTO agent_instructions (
      agent_id, org_id, version, structured_json, markdown, derived_from_json, created_at, updated_at
    ) VALUES (
      "01a20b31-0000-7000-8000-000000000020", "01a20b31-0000-7000-8000-000000000001",
      1, ?, ?, 1, ?, ?
    )
    """, (json.dumps(inst_struct), inst_md, ts, ts))

    c.execute("""
    INSERT INTO knowledge_groups (
      id, org_id, name, description, status, version, created_at, updated_at
    ) VALUES (
      "01a20b31-0000-7000-8000-000000000030", "01a20b31-0000-7000-8000-000000000001",
      "Políticas e Horários", "Informações operacionais e regras de atendimento",
      "active", 1, ?, ?
    )
    """, (ts, ts))

    c.execute("""
    INSERT INTO agent_knowledge_groups (org_id, agent_id, group_id, version, created_at)
    VALUES ("01a20b31-0000-7000-8000-000000000001", "01a20b31-0000-7000-8000-000000000020", "01a20b31-0000-7000-8000-000000000030", 1, ?)
    """, (ts,))

    c.execute("""
    INSERT INTO knowledge_items (
      id, org_id, group_id, title, content, tags_json, source_type, position, status, version, created_at, updated_at
    ) VALUES 
      ("01a20b31-0000-7000-8000-000000000031", "01a20b31-0000-7000-8000-000000000001", "01a20b31-0000-7000-8000-000000000030", "Horário de Funcionamento", "Nosso horário de atendimento é de segunda a sexta-feira, das 8h às 18h.", ?, "manual", 0, "active", 1, ?, ?),
      ("01a20b31-0000-7000-8000-000000000032", "01a20b31-0000-7000-8000-000000000001", "01a20b31-0000-7000-8000-000000000030", "Política de Trocas e Devoluções", "Trocas podem ser solicitadas em até 7 dias corridos após o recebimento do produto.", ?, "manual", 1, "active", 1, ?, ?)
    """, (json.dumps(["horario", "suporte"]), ts, ts, json.dumps(["trocas", "politica"]), ts, ts))

    c.execute("""
    INSERT INTO conversations (
      id, org_id, agent_id, channel, status, version, created_at, updated_at, last_message_at
    ) VALUES (
      "01a20b31-0000-7000-8000-000000000040", "01a20b31-0000-7000-8000-000000000001",
      "01a20b31-0000-7000-8000-000000000020", "test", "open", 1, ?, ?, ?)
    """, (ts, ts + 5000, ts + 5000))

    c.execute("""
    INSERT INTO messages (
      id, org_id, conversation_id, direction, role, text, attachments_json, origin, created_at
    ) VALUES 
      ("01a20b31-0000-7000-8000-000000000041", "01a20b31-0000-7000-8000-000000000001", "01a20b31-0000-7000-8000-000000000040", "in", "user", "Olá! Qual é o horário de atendimento?", "[]", "device", ?),
      ("01a20b31-0000-7000-8000-000000000042", "01a20b31-0000-7000-8000-000000000001", "01a20b31-0000-7000-8000-000000000040", "out", "assistant", "Olá! Nosso horário de atendimento é de segunda a sexta-feira, das 8h às 18h. Posso ajudar com mais alguma dúvida?", "[]", "agent", ?)
    """, (ts, ts + 5000))

    conn.commit()
    conn.close()
    print(f"Banco de demonstração semeado com sucesso em {data_dir}")

if __name__ == "__main__":
    target = sys.argv[1] if len(sys.argv) > 1 else "/tmp/poph-baseline-demo"
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 17480
    setup_demo_data(target, port)
