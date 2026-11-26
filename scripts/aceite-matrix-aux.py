#!/usr/bin/env python3
"""Auxiliar do roteiro de aceite do módulo Matrix (MX-31).

Faz o que o `aceite-matrix.sh` não faz bem em bash: servidores falsos (LLM e Evolution),
chamadas HTTP ao Synapse de desenvolvimento e leitura do estado da sala.

Nenhum token é impresso: os valores vêm do ambiente e só viajam em cabeçalho.
"""
import base64
import hashlib
import json
import os
import struct
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import zlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


_lock = threading.Lock()


def _record(diretorio, nome, data):
    with _lock:
        with open(os.path.join(diretorio, nome), "a", encoding="utf-8") as f:
            f.write(json.dumps(data, ensure_ascii=False) + "\n")


def _llm_response():
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
        size = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(size) or b"{}")
        _record(
            self.diretorio,
            "llm.jsonl",
            {
                "caminho": self.path,
                "modelo": body.get("model"),
                "mensagens": len(body.get("messages") or []),
            },
        )
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()

        def send(obj):
            self.wfile.write(("data: " + json.dumps(obj, ensure_ascii=False) + "\n\n").encode())
            self.wfile.flush()

        send({"choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}, "finish_reason": None}]})
        send({"choices": [{"index": 0, "delta": {"content": _llm_response()}, "finish_reason": None}]})
        send({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "usage": {"prompt_tokens": 10, "completion_tokens": 8}})
        self.wfile.write(b"data: [DONE]\n\n")
        self.wfile.flush()
        self.close_connection = True


def _png_chunk(tipo, dados):
    return (
        struct.pack(">I", len(dados))
        + tipo
        + dados
        + struct.pack(">I", zlib.crc32(tipo + dados) & 0xFFFFFFFF)
    )


def _test_png(width=360, altura=220):
    """PNG RGB com degradê (arquivo pequeno, visível no Element)."""
    lines = []
    for y in range(altura):
        line = bytearray([0])
        for x in range(width):
            line += bytes(((x * 255) // (width - 1), (y * 255) // (altura - 1), 96))
        lines.append(bytes(line))
    ihdr = struct.pack(">IIBBBBB", width, altura, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + _png_chunk(b"IHDR", ihdr)
        + _png_chunk(b"IDAT", zlib.compress(b"".join(lines), 9))
        + _png_chunk(b"IEND", b"")
    )


def _test_pdf(texto="Aceite do modulo Matrix (fase 2) - arquivo de teste"):
    """PDF mínimo de uma página, com uma linha de texto (sem acento, para não depender de fonte)."""
    fluxo = f"BT /F1 14 Tf 40 200 Td ({texto}) Tj ET\n".encode("latin-1")
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 420 260] "
        b"/Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
        b"<< /Length " + str(len(fluxo)).encode() + b" >>\nstream\n" + fluxo + b"endstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]
    saida = bytearray(b"%PDF-1.4\n")
    deslocamentos = []
    for numero, entry in enumerate(objects, start=1):
        deslocamentos.append(len(saida))
        saida += f"{numero} 0 obj\n".encode() + entry + b"\nendobj\n"
    start = len(saida)
    total = len(objects) + 1
    saida += f"xref\n0 {total}\n".encode() + b"0000000000 65535 f \n"
    for deslocamento in deslocamentos:
        saida += f"{deslocamento:010d} 00000 n \n".encode()
    saida += (
        f"trailer\n<< /Size {total} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n".encode()
    )
    return bytes(saida)


def _with_ffmpeg(arguments, suffix):
    """Gera um fixture de áudio/vídeo com o ffmpeg; `None` quando ele não está disponível.

    O Element tenta reproduzir o que está na sala: um arquivo só com o cabeçalho aparece com
    "Erro ao baixar o áudio". Por isso o fixture é real quando há ffmpeg (o caso deste ambiente) e
    cai num cabeçalho mínimo quando não há — o caminho de entrega é o mesmo, o que muda é a
    reprodução na tela.
    """
    import shutil
    import subprocess
    import tempfile

    if shutil.which("ffmpeg") is None:
        return None
    with tempfile.NamedTemporaryFile(suffix=suffix) as saida:
        try:
            subprocess.run(
                ["ffmpeg", "-y", "-loglevel", "error", *arguments, saida.name],
                check=True,
                timeout=60,
            )
        except (OSError, subprocess.SubprocessError):
            return None
        with open(saida.name, "rb") as file:
            return file.read()


def _test_ogg():
    """OGG/Opus de um segundo (silêncio) via ffmpeg; sem ffmpeg, cabeçalho `OggS` mínimo.

    O `infer` reconhece o formato pelo "OggS" nos dois casos; com ffmpeg o Element também
    reproduz o áudio na sala (MX-49).
    """
    real = _with_ffmpeg(
        ["-f", "lavfi", "-i", "anullsrc=r=48000:cl=mono", "-t", "1", "-c:a", "libopus", "-f", "ogg"],
        ".ogg",
    )
    if real:
        return real
    header = (
        b"OggS"
        + b"\x00"
        + b"\x02"
        + b"\x00" * 8
        + b"\x00\x00\x00\x01"
        + b"\x00\x00\x00\x00"
        + b"\x00\x00\x00\x00"
        + b"\x00"
    )
    return header + b"\x00" * 64


def _test_mp4():
    """MP4 H.264 de um segundo via ffmpeg; sem ffmpeg, cabeçalho `ftyp` mínimo (MX-49)."""
    real = _with_ffmpeg(
        [
            "-f",
            "lavfi",
            "-i",
            "color=c=steelblue:s=320x240:d=1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-f",
            "mp4",
        ],
        ".mp4",
    )
    if real:
        return real
    return b"\x00\x00\x00\x18ftypisom\x00\x00\x02\x00isomiso2mp41" + b"\x00" * 64


FIXTURES = {
    "image/png": ("imagem-do-cliente.png", _test_png),
    "application/pdf": ("documento-do-cliente.pdf", _test_pdf),
    "audio/ogg": ("audio-do-cliente.ogg", _test_ogg),
    "video/mp4": ("video-do-cliente.mp4", _test_mp4),
}


def _event_attachment_mimetype(body):
    """Mimetype do evento que o Harness mandou em `{"message": dados}` (ou 404 se desconhecido)."""
    dados = body.get("message") or {}
    content = dados.get("message") or {}
    for chave in ("imageMessage", "documentMessage"):
        block = content.get(chave)
        if isinstance(block, dict):
            return (block.get("mimetype") or "").lower()
    return ""


class _Evo(BaseHTTPRequestHandler):
    """Evolution falsa: texto e mídia enviados, e o download da mídia que o cliente mandou."""

    protocol_version = "HTTP/1.1"
    diretorio = "."

    def log_message(self, *_):
        pass

    def _json(self, code, obj):
        dados = json.dumps(obj, ensure_ascii=False).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(dados)))
        self.end_headers()
        self.wfile.write(dados)

    def do_GET(self):
        if self.path.endswith("/health"):
            return self._json(200, {"status": "ok"})
        return self._json(404, {"message": "nao encontrado"})

    def do_POST(self):
        size = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(size) or b"{}")
        if "/message/sendText/" in self.path:
            instancia = self.path.rsplit("/", 1)[-1]
            _record(
                self.diretorio,
                "evo.jsonl",
                {"instancia": instancia, "number": body.get("number"), "text": body.get("text")},
            )
            return self._json(201, {"key": {"id": "EVO-ACEITE"}, "status": "PENDING"})
        if "/message/sendMedia/" in self.path:
            encoded = body.get("media") or ""
            try:
                attachment_bytes = base64.b64decode(encoded)
            except Exception:  # noqa: BLE001 - prova de ambiente, não de produto
                attachment_bytes = b""
            _record(
                self.diretorio,
                "evo_midia.jsonl",
                {
                    "instancia": self.path.rsplit("/", 1)[-1],
                    "number": body.get("number"),
                    "mediatype": body.get("mediatype"),
                    "mimetype": body.get("mimetype"),
                    "fileName": body.get("fileName"),
                    "caption": body.get("caption"),
                    "bytes": len(attachment_bytes),
                    "sha256": hashlib.sha256(attachment_bytes).hexdigest(),
                },
            )
            return self._json(201, {"key": {"id": "EVO-MIDIA"}, "status": "PENDING"})
        if "/chat/getBase64FromMediaMessage/" in self.path:
            mime = _event_attachment_mimetype(body)
            fixture = FIXTURES.get(mime)
            if fixture is None:
                _record(
                    self.diretorio,
                    "evo_baixadas.jsonl",
                    {"mimetype": mime, "resultado": "nao_servido"},
                )
                return self._json(404, {"status": 404, "error": "Not Found", "message": "Message not found"})
            nome, generate = fixture
            attachment_bytes = generate()
            _record(
                self.diretorio,
                "evo_baixadas.jsonl",
                {
                    "mimetype": mime,
                    "resultado": "servido",
                    "bytes": len(attachment_bytes),
                    "sha256": hashlib.sha256(attachment_bytes).hexdigest(),
                },
            )
            return self._json(
                200,
                {
                    "base64": base64.b64encode(attachment_bytes).decode(),
                    "mimetype": mime,
                    "fileName": nome,
                },
            )
        return self._json(404, {"message": "nao encontrado"})


def _serve(porta, handler_class, diretorio):
    handler_class.diretorio = diretorio
    servidor = ThreadingHTTPServer(("127.0.0.1", porta), handler_class)
    servidor.daemon_threads = True
    servidor.serve_forever()


def fake_commands(diretorio, llm_port, evo_port):
    os.makedirs(diretorio, exist_ok=True)
    threading.Thread(target=_serve, args=(llm_port, _Llm, diretorio), daemon=True).start()
    threading.Thread(target=_serve, args=(evo_port, _Evo, diretorio), daemon=True).start()
    print("falsos no ar", flush=True)
    while True:
        time.sleep(3600)


BASE = os.environ.get("SYNAPSE", "http://127.0.0.1:18008")


def _request(method, path, body=None, token=None):
    dados = None if body is None else json.dumps(body).encode()
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(BASE + path, data=dados, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return resp.status, json.loads(resp.read() or b"{}")
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


def _q(value):
    return urllib.parse.quote(value, safe="")


def _clear_space_children(room, token_as, bot):
    """Tira do espaço as salas de execuções anteriores (o alias é reaproveitado).

    Sem isso, a hierarquia do espaço acumula salas antigas e a captura do aceite fica ilegível.
    Não apaga sala nenhuma: só remove o `m.space.child` do espaço do aceite (artefato de teste).
    O bot do AS é quem escreve: membro do espaço com poder 50 (o atendente novo teria poder 0).
    """
    status, estado = _request("GET", f"/_matrix/client/v3/rooms/{_q(room)}/state", token=token_as)
    if status != 200:
        return 0
    removed = 0
    for evento in estado:
        if evento.get("type") != "m.space.child":
            continue
        child = evento.get("state_key")
        if not child:
            continue
        s, _ = _request(
            "PUT",
            f"/_matrix/client/v3/rooms/{_q(room)}/state/m.space.child/{_q(child)}?user_id={_q(bot)}",
            {},
            token=token_as,
        )
        if s == 200:
            removed += 1
    return removed


def spaces(saida, human_token, as_token, bot):
    """Cria os espaços do aceite como o atendente faria no Element e prepara o bot."""
    os.makedirs(saida, exist_ok=True)
    _, who = _request("GET", "/_matrix/client/v3/account/whoami", token=human_token)
    human = who["user_id"]
    resultado = {}
    for chave, alias, nome in (
        ("comercial", "aceite-comercial", "Comercial"),
        ("suporte", "aceite-suporte", "Suporte"),
    ):
        status, d = _request(
            "POST",
            "/_matrix/client/v3/createRoom",
            {
                "name": nome,
                "room_alias_name": alias,
                "preset": "private_chat",
                "creation_content": {"type": "m.space"},
                "topic": f"Departamento · {nome}",
            },
            token=human_token,
        )
        if status == 400 and d.get("errcode") == "M_ROOM_IN_USE":
            status, d = _request(
                "GET", f"/_matrix/client/v3/directory/room/{_q('#' + alias + ':matrix-dev.local')}"
            )
            room = d["room_id"]
        elif status == 200:
            room = d["room_id"]
        else:
            raise SystemExit(f"createRoom do espaço {alias} falhou: {status} {d}")
        _request("POST", f"/_matrix/client/v3/rooms/{_q(room)}/invite", {"user_id": bot}, token=human_token)
        _request("POST", f"/_matrix/client/v3/rooms/{_q(room)}/invite", {"user_id": human}, token=as_token)
        _request("POST", f"/_matrix/client/v3/rooms/{_q(room)}/join", {}, token=human_token)
        _request(
            "POST",
            f"/_matrix/client/v3/rooms/{_q(room)}/join?user_id={_q(bot)}",
            {},
            token=as_token,
        )
        status, power = _request("GET", f"/_matrix/client/v3/rooms/{_q(room)}/state/m.room.power_levels", token=human_token)
        if status == 200:
            power.setdefault("users", {})[bot] = 50
            _request("PUT", f"/_matrix/client/v3/rooms/{_q(room)}/state/m.room.power_levels", power, token=human_token)
        removed = _clear_space_children(room, as_token, bot)
        resultado[chave] = {
            "sala": room,
            "alias": f"#{alias}:matrix-dev.local",
            "nome": nome,
            "filhos_removidos": removed,
        }
    with open(os.path.join(saida, "espacos.json"), "w", encoding="utf-8") as f:
        json.dump(resultado, f, indent=2)
    print(json.dumps(resultado, ensure_ascii=False))


def webhook(org, instancia, secret, id_externo, texto, push_name):
    base = os.environ.get("HARNESS", "http://127.0.0.1:18010")
    body = {
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
        data=json.dumps(body).encode(),
        method="POST",
        headers={"Content-Type": "application/json", "X-Evolution-Webhook-Secret": secret},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        print(resp.status, json.loads(resp.read()))


def join_room(room):
    base = os.environ.get("SYNAPSE", "http://127.0.0.1:18008")
    status, d = _request("POST", f"/_matrix/client/v3/rooms/{_q(room)}/join", {}, token=os.environ["ACEITE_TOKEN_HUMANO"])
    print(status, d)


def agent_send(room, texto, token):
    base = os.environ.get("SYNAPSE", "http://127.0.0.1:18008")
    txn = f"aceite-{int(time.time() * 1000)}"
    status, d = _request(
        "PUT",
        f"/_matrix/client/v3/rooms/{_q(room)}/send/m.room.message/{txn}",
        {"msgtype": "m.text", "body": texto},
        token=token,
    )
    print(status, d)


def room_state(room, token):
    """Estado relevante da sala, para o relatório e as checagens do roteiro."""
    saida = {}
    for evento in ("m.room.name", "com.sofacoding.atendimento.metadata", "m.space.parent", "m.room.join_rules"):
        status, d = _request("GET", f"/_matrix/client/v3/rooms/{_q(room)}/state/{evento}", token=token)
        saida[evento] = d if status == 200 else {"_status": status}
    status, d = _request("GET", f"/_matrix/client/v3/rooms/{_q(room)}/joined_members", token=token)
    saida["membros"] = sorted(d.get("joined", {})) if status == 200 else {"_status": status}
    print(json.dumps(saida, ensure_ascii=False, indent=2))


def hierarquia(espaco, token):
    base = os.environ.get("SYNAPSE", "http://127.0.0.1:18008")
    status, d = _request("GET", f"/_matrix/client/v1/rooms/{_q(espaco)}/hierarchy?limit=20", token=token)
    if status != 200:
        print(json.dumps({"status": status, "erro": d}))
        return
    print(json.dumps([{"room_id": r.get("room_id"), "name": r.get("name")} for r in d.get("rooms", [])], ensure_ascii=False))


def space_parent(room, espaco, token):
    status, d = _request("GET", f"/_matrix/client/v3/rooms/{_q(espaco)}/state/m.space.child/{_q(room)}", token=token)
    print(json.dumps(d if status == 200 else {"_status": status, "_erro": d}))


def room_parent(room, token):
    status, d = _request("GET", f"/_matrix/client/v3/rooms/{_q(room)}/state/m.space.parent", token=token)
    print(json.dumps(d if status == 200 else {"_status": status, "_erro": d}))


def notices(room, token):
    """Conta os `m.notice` do histórico da sala (prova de que renomear não avisa)."""
    status, d = _request("GET", f"/_matrix/client/v3/rooms/{_q(room)}/messages?dir=b&limit=100", token=token)
    if status != 200:
        print(json.dumps({"status": status, "erro": d}, ensure_ascii=False))
        return
    kinds = [e for e in d.get("chunk", []) if e.get("type") == "m.notice"]
    print(
        json.dumps(
            {"total": len(kinds), "corpos": [e.get("content", {}).get("body") for e in kinds]},
            ensure_ascii=False,
        )
    )


def profile(user, token):
    """Nome de exibição de um fantasma (MX-10/MX-33)."""
    status, d = _request("GET", f"/_matrix/client/v3/profile/{_q(user)}/displayname", token=token)
    print(json.dumps(d if status == 200 else {"_status": status}, ensure_ascii=False))


def play(db, conversa):
    """Sobe `updated_at` da conversa, como qualquer alteração real faria (é o gatilho do laço de estado)."""
    import sqlite3

    con = sqlite3.connect(db)
    now = con.execute("select coalesce(max(updated_at),0)+1 from conversations").fetchone()[0]
    con.execute("update conversations set updated_at=? where id=?", (now, conversa))
    con.commit()
    print(f"conversa {conversa} tocada (updated_at {now})")


def transfer_department(db, conversa, departamento):
    """Simula a transferência da MX-15: muda o alvo e sobe `updated_at` (o gatilho real é o G1/Jev)."""
    import sqlite3

    con = sqlite3.connect(db)
    now = int(time.time() * 1000)
    con.execute(
        "update conversations set target_department_id=?, updated_at=? where id=?",
        (departamento, now, conversa),
    )
    con.commit()
    print(f"conversa {conversa} -> departamento {departamento} (updated_at {now})")


def _request_raw(method, path, dados=None, token=None, content_type="application/octet-stream"):
    """Pedido com corpo cru (upload de mídia). Devolve status e corpo JSON (ou vazio)."""
    headers = {"Content-Type": content_type}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(BASE + path, data=dados, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            raw = resp.read()
            return resp.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as e:
        raw = e.read()
        try:
            body = json.loads(raw or b"{}")
        except ValueError:
            body = {}
        return e.code, body


def webhook_attachment(org, instancia, id_externo, tipo, caption, push_name):
    """Cliente manda imagem ou documento pelo WhatsApp (MX-43).

    A mensagem leva só a referência (mimetype e legenda); o conteúdo o Harness baixa da
    Evolution falsa, que serve o fixture do tipo.
    """
    base = os.environ.get("HARNESS", "http://127.0.0.1:18010")
    phone = os.environ.get("ACEITE_TELEFONE", "5521988887777")
    if tipo == "imagem":
        content = {
            "imageMessage": {"mimetype": "image/png", "caption": caption, "fileLength": 4096}
        }
        message_type = "imageMessage"
    else:
        content = {
            "documentMessage": {
                "mimetype": "application/pdf",
                "caption": caption,
                "fileName": "documento-do-cliente.pdf",
                "fileLength": 2048,
            }
        }
        message_type = "documentMessage"
    body = {
        "event": "messages.upsert",
        "instance": instancia,
        "apikey": "chave-falsa",
        "data": {
            "key": {
                "id": id_externo,
                "remoteJid": phone + "@s.whatsapp.net",
                "fromMe": False,
            },
            "pushName": push_name,
            "messageType": message_type,
            "message": content,
            "messageTimestamp": int(time.time()),
        },
    }
    req = urllib.request.Request(
        f"{base}/webhook/evolution/{org}/{instancia}",
        data=json.dumps(body).encode(),
        method="POST",
        headers={
            "Content-Type": "application/json",
            "X-Evolution-Webhook-Secret": os.environ["ACEITE_SEGREDO_WEBHOOK"],
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        print(resp.status, json.loads(resp.read()))


OUT_OF_SCOPE_KINDS = {
    "audio": ("audioMessage", {"ptt": True, "seconds": 4}),
    "video": ("videoMessage", {"seconds": 3}),
    "figurinha": ("stickerMessage", {"isAnimated": False}),
    "contato": ("contactMessage", {"displayName": "Contato de teste"}),
    "localizacao": ("locationMessage", {"degreesLatitude": -22.9, "degreesLongitude": -43.2}),
    "enquete": ("pollCreationMessage", {"name": "Enquete de teste", "selectableOptionsCount": 2}),
}


def write_fixtures(diretorio):
    """Grava o PNG e o PDF de teste (usados no envio do atendente, MX-44) e imprime os caminhos."""
    os.makedirs(diretorio, exist_ok=True)
    paths = {}
    for mime, (nome, generate) in FIXTURES.items():
        path = os.path.join(diretorio, nome.replace("do-cliente", "do-atendente"))
        with open(path, "wb") as file:
            file.write(generate())
        paths[mime] = path
    print(json.dumps(paths, ensure_ascii=False))


def webhook_type(org, instancia, id_externo, tipo, push_name):
    """Cliente manda um tipo que o Harness não abre (áudio, vídeo, figurinha, contato,
    localização, enquete): vira texto de histórico no turno e aviso na sala (MX-45)."""
    base = os.environ.get("HARNESS", "http://127.0.0.1:18010")
    phone = os.environ.get("ACEITE_TELEFONE", "5521988887777")
    if tipo not in OUT_OF_SCOPE_KINDS:
        raise SystemExit(f"tipo fora do escopo desconhecido: {tipo}")
    chave, content = OUT_OF_SCOPE_KINDS[tipo]
    body = {
        "event": "messages.upsert",
        "instance": instancia,
        "apikey": "chave-falsa",
        "data": {
            "key": {
                "id": id_externo,
                "remoteJid": phone + "@s.whatsapp.net",
                "fromMe": False,
            },
            "pushName": push_name,
            "messageType": chave,
            "message": {chave: content},
            "messageTimestamp": int(time.time()),
        },
    }
    req = urllib.request.Request(
        f"{base}/webhook/evolution/{org}/{instancia}",
        data=json.dumps(body).encode(),
        method="POST",
        headers={
            "Content-Type": "application/json",
            "X-Evolution-Webhook-Secret": os.environ["ACEITE_SEGREDO_WEBHOOK"],
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        print(resp.status, json.loads(resp.read()))


def webhook_device(org, instancia, id_externo, texto):
    """Mensagem digitada no celular da loja (`fromMe`): MX-41/MX-42."""
    base = os.environ.get("HARNESS", "http://127.0.0.1:18010")
    phone = os.environ.get("ACEITE_TELEFONE", "5521988887777")
    body = {
        "event": "messages.upsert",
        "instance": instancia,
        "apikey": "chave-falsa",
        "data": {
            "key": {
                "id": id_externo,
                "remoteJid": phone + "@s.whatsapp.net",
                "fromMe": True,
            },
            "pushName": "Loja",
            "messageType": "conversation",
            "message": {"conversation": texto},
            "messageTimestamp": int(time.time()),
        },
    }
    req = urllib.request.Request(
        f"{base}/webhook/evolution/{org}/{instancia}",
        data=json.dumps(body).encode(),
        method="POST",
        headers={
            "Content-Type": "application/json",
            "X-Evolution-Webhook-Secret": os.environ["ACEITE_SEGREDO_WEBHOOK"],
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        print(resp.status, json.loads(resp.read()))


def agent_send_attachment(room, path, mime, nome, txn=None):
    """Atendente manda imagem, arquivo, áudio ou vídeo pela sala (o evento que o Element publica).

    O `msgtype` segue o tipo real, como o Element faz: `m.image`, `m.audio`, `m.video` e `m.file`
    para o resto (MX-44 e MX-49). O `txn` é opcional: repetir o mesmo valor reenvia a mesma
    transação do Element, que o Synapse deduplica (prova de reentrega do MX-49).
    """
    token = os.environ["ACEITE_TOKEN_HUMANO"]
    with open(path, "rb") as file:
        dados = file.read()
    status, d = _request_raw(
        "POST",
        f"/_matrix/media/v3/upload?filename={_q(nome)}",
        dados,
        token=token,
        content_type=mime,
    )
    if status != 200 or "content_uri" not in d:
        print(status, json.dumps(d, ensure_ascii=False))
        return
    mxc = d["content_uri"]
    if mime.startswith("image/"):
        msgtype = "m.image"
    elif mime.startswith("audio/"):
        msgtype = "m.audio"
    elif mime.startswith("video/"):
        msgtype = "m.video"
    else:
        msgtype = "m.file"
    content = {
        "msgtype": msgtype,
        "body": nome,
        "filename": nome,
        "url": mxc,
        "info": {"mimetype": mime, "size": len(dados)},
    }
    txn = txn or f"aceite-midia-{int(time.time() * 1000)}"
    status, d = _request(
        "PUT",
        f"/_matrix/client/v3/rooms/{_q(room)}/send/m.room.message/{txn}",
        content,
        token=token,
    )
    print(status, json.dumps(d, ensure_ascii=False))


def events(room, limit=80):
    """Timeline da sala em forma de relatório: corpo, remetente, anexo e redação.

    Só o que é preciso para as asserções: nada de token, e o texto que aparece é o do
    roteiro (fixtures), nunca dado de cliente real.
    """
    token = os.environ["ACEITE_TOKEN_HUMANO"]
    status, d = _request(
        "GET",
        f"/_matrix/client/v3/rooms/{_q(room)}/messages?dir=b&limit={int(limit)}",
        token=token,
    )
    if status != 200:
        print(json.dumps({"status": status, "erro": d}, ensure_ascii=False))
        return
    items = []
    for evento in d.get("chunk", []):
        if evento.get("type") != "m.room.message":
            continue
        content = evento.get("content") or {}
        file = content.get("file") if isinstance(content.get("file"), dict) else {}
        block = {
            chave: value
            for chave, value in content.items()
            if chave.startswith("com.") and chave.endswith(".attachment")
        }
        item = {
            "evento": evento.get("event_id"),
            "remetente": evento.get("sender"),
            "msgtype": content.get("msgtype"),
            "corpo": content.get("body"),
            "nome": content.get("filename") or file.get("filename"),
            "mxc": content.get("url") or file.get("url"),
            "bloco_de_anexo": block,
            "redigido": bool((evento.get("unsigned") or {}).get("redacted_because")),
        }
        redaction = (evento.get("unsigned") or {}).get("redacted_because") or {}
        if redaction:
            redaction_content = redaction.get("content") or {}
            item["redigido_por"] = redaction.get("sender")
            item["bloco_expirado"] = {
                chave: value
                for chave, value in redaction_content.items()
                if chave.endswith(".attachment_expired")
            }
        items.append(item)
    print(json.dumps(items, ensure_ascii=False, indent=2))


def evento(room, event_id):
    token = os.environ["ACEITE_TOKEN_HUMANO"]
    status, d = _request(
        "GET",
        f"/_matrix/client/v3/rooms/{_q(room)}/event/{_q(event_id)}",
        token=token,
    )
    print(json.dumps({"status": status, "corpo": d}, ensure_ascii=False, indent=2))


def attachment_exists(mxc):
    """O arquivo responde no download de mídia do Synapse? (200 existe, 404 apagado)"""
    token = os.environ["ACEITE_TOKEN_HUMANO"]
    if not mxc.startswith("mxc://"):
        print(json.dumps({"erro": "endereco nao e mxc"}))
        return
    servidor, identifier = mxc[len("mxc://") :].split("/", 1)
    status, d = _request(
        "GET",
        f"/_matrix/client/v1/media/download/{_q(servidor)}/{_q(identifier)}",
        token=token,
    )
    print(json.dumps({"status": status, "erro": d.get("errcode")}, ensure_ascii=False))


def prepare_expiry(db, dias):
    """Recua o `sent_at` da mídia pendente, para o prazo vencer sem esperar dias (só no teste)."""
    import sqlite3

    cutoff = int(time.time() * 1000) - int(dias) * 86_400_000 - 60_000
    con = sqlite3.connect(db)
    lines = con.execute(
        "select count(*) from matrix_midia where redacted_at is null or deleted_at is null"
    ).fetchone()[0]
    con.execute(
        "update matrix_midia set sent_at=?, updated_at=? where redacted_at is null or deleted_at is null",
        (cutoff, int(time.time() * 1000)),
    )
    con.commit()
    print(json.dumps({"adiantadas": lines, "sent_at": cutoff}))


def expiry_state(db):
    import sqlite3

    con = sqlite3.connect(db)
    try:
        lines = con.execute(
            "select id, event_id, mxc, kind, name, sent_at, redacted_at, deleted_at,"
            " attempts, next_attempt_at, coalesce(last_error,'') from matrix_midia order by sent_at"
        ).fetchall()
    except sqlite3.OperationalError as error:
        print(json.dumps({"erro": str(error)}))
        return
    print(
        json.dumps(
            [
                {
                    "id": line[0],
                    "evento": line[1],
                    "mxc": line[2],
                    "categoria": line[3],
                    "nome": line[4],
                    "sent_at": line[5],
                    "redacted_at": line[6],
                    "deleted_at": line[7],
                    "tentativas": line[8],
                    "proxima_tentativa": line[9],
                    "ultimo_erro": line[10],
                }
                for line in lines
            ],
            ensure_ascii=False,
            indent=2,
        )
    )


def main(argv):
    if len(argv) < 2:
        raise SystemExit("uso: aceite-matrix-aux.py <comando> ...")
    command = argv[1]
    if command == "falsos":
        fake_commands(argv[2], int(argv[3]), int(argv[4]))
    elif command == "espacos":
        spaces(argv[2], os.environ["ACEITE_TOKEN_HUMANO"], os.environ["ACEITE_AS_TOKEN"], os.environ.get("ACEITE_BOT", "@ph_as_bot:matrix-dev.local"))
    elif command == "webhook":
        webhook(argv[2], argv[3], os.environ["ACEITE_SEGREDO_WEBHOOK"], argv[4], argv[5], argv[6])
    elif command == "entrar":
        join_room(argv[2])
    elif command == "atendente-envia":
        agent_send(argv[2], argv[3], os.environ["ACEITE_TOKEN_HUMANO"])
    elif command == "estado-sala":
        room_state(argv[2], os.environ["ACEITE_TOKEN_HUMANO"])
    elif command == "hierarquia":
        hierarquia(argv[2], os.environ["ACEITE_TOKEN_HUMANO"])
    elif command == "pai-do-espaco":
        space_parent(argv[2], argv[3], os.environ["ACEITE_TOKEN_HUMANO"])
    elif command == "pai-da-sala":
        room_parent(argv[2], os.environ["ACEITE_TOKEN_HUMANO"])
    elif command == "avisos":
        notices(argv[2], os.environ["ACEITE_TOKEN_HUMANO"])
    elif command == "perfil":
        profile(argv[2], os.environ["ACEITE_TOKEN_HUMANO"])
    elif command == "tocar":
        play(argv[2], argv[3])
    elif command == "transferir":
        transfer_department(argv[2], argv[3], argv[4])
    elif command == "webhook-midia":
        webhook_attachment(argv[2], argv[3], argv[4], argv[5], argv[6], argv[7])
    elif command == "webhook-aparelho":
        webhook_device(argv[2], argv[3], argv[4], argv[5])
    elif command == "webhook-tipo":
        webhook_type(argv[2], argv[3], argv[4], argv[5], argv[6])
    elif command == "atendente-envia-midia":
        agent_send_attachment(argv[2], argv[3], argv[4], argv[5], argv[6] if len(argv) > 6 else None)
    elif command == "eventos":
        events(argv[2], argv[3] if len(argv) > 3 else 80)
    elif command == "evento":
        evento(argv[2], argv[3])
    elif command == "midia-existe":
        attachment_exists(argv[2])
    elif command == "preparar-expiracao":
        prepare_expiry(argv[2], argv[3])
    elif command == "estado-expiracao":
        expiry_state(argv[2])
    elif command == "fixtures":
        write_fixtures(argv[2])
    else:
        raise SystemExit(f"comando desconhecido: {command}")


if __name__ == "__main__":
    main(sys.argv)
