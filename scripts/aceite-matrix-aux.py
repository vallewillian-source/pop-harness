#!/usr/bin/env python3
"""Auxiliar do roteiro de aceite do módulo Matrix (MX-31).

Faz o que o `aceite-matrix.sh` não faz bem em bash: servidores falsos (LLM e Evolution),
chamadas HTTP ao Synapse de desenvolvimento e leitura do estado da sala.

Nenhum token é impresso: os valores vêm do ambiente e só viajam em cabeçalho.
"""
import json
import os
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

# ───────────────────────────── falsos (LLM e Evolution) ─────────────────────────────

_trava = threading.Lock()


def _registrar(diretorio, nome, dado):
    with _trava:
        with open(os.path.join(diretorio, nome), "a", encoding="utf-8") as f:
            f.write(json.dumps(dado, ensure_ascii=False) + "\n")


def _resposta_do_llm():
    return os.environ.get(
        "ACEITE_RESPOSTA", "Olá! Sou a Ana, do time Comercial. Como posso ajudar?"
    )


class _Llm(BaseHTTPRequestHandler):
    """OpenAI compatível, SSE: o Harness fala com o antigravity2api por stream."""

    protocol_version = "HTTP/1.1"
    diretorio = "."

    def log_message(self, *_):
        pass

    def do_POST(self):
        tamanho = int(self.headers.get("Content-Length") or 0)
        corpo = json.loads(self.rfile.read(tamanho) or b"{}")
        _registrar(
            self.diretorio,
            "llm.jsonl",
            {
                "caminho": self.path,
                "modelo": corpo.get("model"),
                "mensagens": len(corpo.get("messages") or []),
            },
        )
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()

        def envia(obj):
            self.wfile.write(("data: " + json.dumps(obj, ensure_ascii=False) + "\n\n").encode())
            self.wfile.flush()

        envia({"choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": None}]})
        envia({"choices": [{"index": 0, "delta": {"content": _resposta_do_llm()}, "finish_reason": None}]})
        envia({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 10, "completion_tokens": 8}})
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()
        self.close_connection = True


class _Evo(BaseHTTPRequestHandler):
    """Evolution falsa: só o envio de texto da fase 1 (e o download de mídia, que recusa)."""

    protocol_version = "HTTP/1.1"
    diretorio = "."

    def log_message(self, *_):
        pass

    def _json(self, codigo, obj):
        dados = json.dumps(obj, ensure_ascii=False).encode()
        self.send_response(codigo)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(dados)))
        self.end_headers()
        self.wfile.write(dados)

    def do_GET(self):
        if self.path.endswith("/health"):
            return self._json(200, {"status": "ok"})
        return self._json(404, {"message": "nao encontrado"})

    def do_POST(self):
        tamanho = int(self.headers.get("Content-Length") or 0)
        corpo = json.loads(self.rfile.read(tamanho) or b"{}")
        if "/message/sendText/" in self.path:
            instancia = self.path.rsplit("/", 1)[-1]
            _registrar(
                self.diretorio,
                "evo.jsonl",
                {"instancia": instancia, "number": corpo.get("number"), "text": corpo.get("text")},
            )
            return self._json(201, {"key": {"id": "EVO-ACEITE"}, "status": "PENDING"})
        if "/message/sendMedia/" in self.path:
            _registrar(self.diretorio, "evo_midia.jsonl", {"instancia": self.path.rsplit("/", 1)[-1]})
            return self._json(201, {"key": {"id": "EVO-MIDIA"}, "status": "PENDING"})
        if "/chat/getBase64FromMediaMessage/" in self.path:
            return self._json(404, {"status": 404, "error": "Not Found", "message": "Message not found"})
        return self._json(404, {"message": "nao encontrado"})


def _servir(porta, classe, diretorio):
    classe.diretorio = diretorio
    servidor = ThreadingHTTPServer(("127.0.0.1", porta), classe)
    servidor.daemon_threads = True
    servidor.serve_forever()


def comandos_falsos(diretorio, porta_llm, porta_evo):
    os.makedirs(diretorio, exist_ok=True)
    threading.Thread(target=_servir, args=(porta_llm, _Llm, diretorio), daemon=True).start()
    threading.Thread(target=_servir, args=(porta_evo, _Evo, diretorio), daemon=True).start()
    print("falsos no ar", flush=True)
    while True:
        time.sleep(3600)


# ───────────────────────────── Synapse ─────────────────────────────

BASE = os.environ.get("SYNAPSE", "http://127.0.0.1:18008")


def _pedir(metodo, caminho, corpo=None, token=None):
    dados = None if corpo is None else json.dumps(corpo).encode()
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(BASE + caminho, data=dados, method=metodo, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return resp.status, json.loads(resp.read() or b"{}")
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


def _q(valor):
    return urllib.parse.quote(valor, safe="")


def _limpar_filhos_do_espaco(sala, token_as, bot):
    """Tira do espaço as salas de execuções anteriores (o alias é reaproveitado).

    Sem isso, a hierarquia do espaço acumula salas antigas e a captura do aceite fica ilegível.
    Não apaga sala nenhuma: só remove o `m.space.child` do espaço do aceite (artefato de teste).
    O bot do AS é quem escreve: membro do espaço com poder 50 (o atendente novo teria poder 0).
    """
    status, estado = _pedir("GET", f"/_matrix/client/v3/rooms/{_q(sala)}/state", token=token_as)
    if status != 200:
        return 0
    removidos = 0
    for evento in estado:
        if evento.get("type") != "m.space.child":
            continue
        filho = evento.get("state_key")
        if not filho:
            continue
        s, _ = _pedir(
            "PUT",
            f"/_matrix/client/v3/rooms/{_q(sala)}/state/m.space.child/{_q(filho)}?user_id={_q(bot)}",
            {},
            token=token_as,
        )
        if s == 200:
            removidos += 1
    return removidos


def espacos(saida, token_humano, as_token, bot):
    """Cria os espaços do aceite como o atendente faria no Element e prepara o bot."""
    os.makedirs(saida, exist_ok=True)
    _, quem = _pedir("GET", "/_matrix/client/v3/account/whoami", token=token_humano)
    humano = quem["user_id"]
    resultado = {}
    for chave, alias, nome in (
        ("comercial", "aceite-comercial", "Comercial"),
        ("suporte", "aceite-suporte", "Suporte"),
    ):
        status, d = _pedir(
            "POST",
            "/_matrix/client/v3/createRoom",
            {
                "name": nome,
                "room_alias_name": alias,
                "preset": "private_chat",
                "creation_content": {"type": "m.space"},
                "topic": f"Departamento · {nome}",
            },
            token=token_humano,
        )
        if status == 400 and d.get("errcode") == "M_ROOM_IN_USE":
            status, d = _pedir(
                "GET", f"/_matrix/client/v3/directory/room/{_q('#' + alias + ':matrix-dev.local')}"
            )
            sala = d["room_id"]
        elif status == 200:
            sala = d["room_id"]
        else:
            raise SystemExit(f"createRoom do espaço {alias} falhou: {status} {d}")
        _pedir("POST", f"/_matrix/client/v3/rooms/{_q(sala)}/invite", {"user_id": bot}, token=token_humano)
        # Space de execução anterior: o atendente novo não é membro. O bot (poder 50) convida e ele entra.
        _pedir("POST", f"/_matrix/client/v3/rooms/{_q(sala)}/invite", {"user_id": humano}, token=as_token)
        _pedir("POST", f"/_matrix/client/v3/rooms/{_q(sala)}/join", {}, token=token_humano)
        _pedir(
            "POST",
            f"/_matrix/client/v3/rooms/{_q(sala)}/join?user_id={_q(bot)}",
            {},
            token=as_token,
        )
        status, poder = _pedir("GET", f"/_matrix/client/v3/rooms/{_q(sala)}/state/m.room.power_levels", token=token_humano)
        if status == 200:
            poder.setdefault("users", {})[bot] = 50
            _pedir("PUT", f"/_matrix/client/v3/rooms/{_q(sala)}/state/m.room.power_levels", poder, token=token_humano)
        removidos = _limpar_filhos_do_espaco(sala, as_token, bot)
        resultado[chave] = {
            "sala": sala,
            "alias": f"#{alias}:matrix-dev.local",
            "nome": nome,
            "filhos_removidos": removidos,
        }
    with open(os.path.join(saida, "espacos.json"), "w", encoding="utf-8") as f:
        json.dump(resultado, f, indent=2)
    print(json.dumps(resultado, ensure_ascii=False))


def webhook(org, instancia, segredo, id_externo, texto, push_name):
    base = os.environ.get("HARNESS", "http://127.0.0.1:18010")
    corpo = {
        "event": "messages.upsert",
        "instance": instancia,
        "apikey": "chave-falsa",
        "data": {
            "key": {"id": id_externo, "remoteJid": os.environ.get("ACEITE_TELEFONE", "5521988887777") + "@s.whatsapp.net", "fromMe": False},
            "pushName": push_name,
            "messageType": "conversation",
            "message": {"conversation": texto},
            "messageTimestamp": int(time.time()),
        },
    }
    req = urllib.request.Request(
        f"{base}/webhook/evolution/{org}/{instancia}",
        data=json.dumps(corpo).encode(),
        method="POST",
        headers={"Content-Type": "application/json", "X-Evolution-Webhook-Secret": segredo},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        print(resp.status, json.loads(resp.read()))


def entrar(sala):
    base = os.environ.get("SYNAPSE", "http://127.0.0.1:18008")
    status, d = _pedir("POST", f"/_matrix/client/v3/rooms/{_q(sala)}/join", {}, token=os.environ["ACEITE_TOKEN_HUMANO"])
    print(status, d)


def atendente_envia(sala, texto, token):
    base = os.environ.get("SYNAPSE", "http://127.0.0.1:18008")
    txn = f"aceite-{int(time.time() * 1000)}"
    status, d = _pedir(
        "PUT",
        f"/_matrix/client/v3/rooms/{_q(sala)}/send/m.room.message/{txn}",
        {"msgtype": "m.text", "body": texto},
        token=token,
    )
    print(status, d)


def estado_sala(sala, token):
    """Estado relevante da sala, para o relatório e as checagens do roteiro."""
    saida = {}
    for evento in ("m.room.name", "com.sofacoding.atendimento.metadata", "m.space.parent", "m.room.join_rules"):
        status, d = _pedir("GET", f"/_matrix/client/v3/rooms/{_q(sala)}/state/{evento}", token=token)
        saida[evento] = d if status == 200 else {"_status": status}
    status, d = _pedir("GET", f"/_matrix/client/v3/rooms/{_q(sala)}/joined_members", token=token)
    saida["membros"] = sorted(d.get("joined", {})) if status == 200 else {"_status": status}
    print(json.dumps(saida, ensure_ascii=False, indent=2))


def hierarquia(espaco, token):
    base = os.environ.get("SYNAPSE", "http://127.0.0.1:18008")
    status, d = _pedir("GET", f"/_matrix/client/v1/rooms/{_q(espaco)}/hierarchy?limit=20", token=token)
    if status != 200:
        print(json.dumps({"status": status, "erro": d}))
        return
    print(json.dumps([{"room_id": r.get("room_id"), "name": r.get("name")} for r in d.get("rooms", [])], ensure_ascii=False))


def pai_do_espaco(sala, espaco, token):
    status, d = _pedir("GET", f"/_matrix/client/v3/rooms/{_q(espaco)}/state/m.space.child/{_q(sala)}", token=token)
    print(json.dumps(d if status == 200 else {"_status": status, "_erro": d}))


def pai_da_sala(sala, token):
    status, d = _pedir("GET", f"/_matrix/client/v3/rooms/{_q(sala)}/state/m.space.parent", token=token)
    print(json.dumps(d if status == 200 else {"_status": status, "_erro": d}))


def avisos(sala, token):
    """Conta os `m.notice` do histórico da sala (prova de que renomear não avisa)."""
    status, d = _pedir("GET", f"/_matrix/client/v3/rooms/{_q(sala)}/messages?dir=b&limit=100", token=token)
    if status != 200:
        print(json.dumps({"status": status, "erro": d}, ensure_ascii=False))
        return
    tipos = [e for e in d.get("chunk", []) if e.get("type") == "m.notice"]
    print(
        json.dumps(
            {"total": len(tipos), "corpos": [e.get("content", {}).get("body") for e in tipos]},
            ensure_ascii=False,
        )
    )


def perfil(usuario, token):
    """Nome de exibição de um fantasma (MX-10/MX-33)."""
    status, d = _pedir("GET", f"/_matrix/client/v3/profile/{_q(usuario)}/displayname", token=token)
    print(json.dumps(d if status == 200 else {"_status": status}, ensure_ascii=False))


def tocar(banco, conversa):
    """Sobe `updated_at` da conversa, como qualquer alteração real faria (é o gatilho do laço de estado)."""
    import sqlite3

    con = sqlite3.connect(banco)
    # Precisa ser maior que o maior `updated_at` já visto pelo cursor (par updated_at, id).
    agora = con.execute("select coalesce(max(updated_at),0)+1 from conversations").fetchone()[0]
    con.execute("update conversations set updated_at=? where id=?", (agora, conversa))
    con.commit()
    print(f"conversa {conversa} tocada (updated_at {agora})")


def transferir_departamento(banco, conversa, departamento):
    """Simula a transferência da MX-15: muda o alvo e sobe `updated_at` (o gatilho real é o G1/Jev)."""
    import sqlite3

    con = sqlite3.connect(banco)
    agora = int(time.time() * 1000)
    con.execute(
        "update conversations set target_department_id=?, updated_at=? where id=?",
        (departamento, agora, conversa),
    )
    con.commit()
    print(f"conversa {conversa} -> departamento {departamento} (updated_at {agora})")


def main(argv):
    if len(argv) < 2:
        raise SystemExit("uso: aceite-matrix-aux.py <comando> ...")
    comando = argv[1]
    if comando == "falsos":
        comandos_falsos(argv[2], int(argv[3]), int(argv[4]))
    elif comando == "espacos":
        espacos(argv[2], os.environ["ACEITE_TOKEN_HUMANO"], os.environ["ACEITE_AS_TOKEN"], os.environ.get("ACEITE_BOT", "@ph_as_bot:matrix-dev.local"))
    elif comando == "webhook":
        webhook(argv[2], argv[3], os.environ["ACEITE_SEGREDO_WEBHOOK"], argv[4], argv[5], argv[6])
    elif comando == "entrar":
        entrar(argv[2])
    elif comando == "atendente-envia":
        atendente_envia(argv[2], argv[3], os.environ["ACEITE_TOKEN_HUMANO"])
    elif comando == "estado-sala":
        estado_sala(argv[2], os.environ["ACEITE_TOKEN_HUMANO"])
    elif comando == "hierarquia":
        hierarquia(argv[2], os.environ["ACEITE_TOKEN_HUMANO"])
    elif comando == "pai-do-espaco":
        pai_do_espaco(argv[2], argv[3], os.environ["ACEITE_TOKEN_HUMANO"])
    elif comando == "pai-da-sala":
        pai_da_sala(argv[2], os.environ["ACEITE_TOKEN_HUMANO"])
    elif comando == "avisos":
        avisos(argv[2], os.environ["ACEITE_TOKEN_HUMANO"])
    elif comando == "perfil":
        perfil(argv[2], os.environ["ACEITE_TOKEN_HUMANO"])
    elif comando == "tocar":
        tocar(argv[2], argv[3])
    elif comando == "transferir":
        transferir_departamento(argv[2], argv[3], argv[4])
    else:
        raise SystemExit(f"comando desconhecido: {comando}")


if __name__ == "__main__":
    main(sys.argv)
