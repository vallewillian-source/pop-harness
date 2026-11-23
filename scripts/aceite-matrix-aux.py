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


# ─────────────── fixtures de mídia da fase 2 (cliente → sala, MX-43) ───────────────
#
# O webhook manda só a referência da mídia; o Harness baixa o conteúdo pela Evolution
# (`/chat/getBase64FromMediaMessage/{instância}`). A Evolution falsa serve um PNG e um PDF
# gerados aqui — sem dependência de PIL, mesmo conteúdo em toda execução — e nunca dado de
# cliente. O tipo vem do `mimetype` declarado no evento, como o Harness espera.


def _bloco_png(tipo, dados):
    return (
        struct.pack(">I", len(dados))
        + tipo
        + dados
        + struct.pack(">I", zlib.crc32(tipo + dados) & 0xFFFFFFFF)
    )


def _png_de_teste(largura=360, altura=220):
    """PNG RGB com degradê (arquivo pequeno, visível no Element)."""
    linhas = []
    for y in range(altura):
        linha = bytearray([0])  # filtro "None" de cada linha
        for x in range(largura):
            linha += bytes(((x * 255) // (largura - 1), (y * 255) // (altura - 1), 96))
        linhas.append(bytes(linha))
    ihdr = struct.pack(">IIBBBBB", largura, altura, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + _bloco_png(b"IHDR", ihdr)
        + _bloco_png(b"IDAT", zlib.compress(b"".join(linhas), 9))
        + _bloco_png(b"IEND", b"")
    )


def _pdf_de_teste(texto="Aceite do modulo Matrix (fase 2) - arquivo de teste"):
    """PDF mínimo de uma página, com uma linha de texto (sem acento, para não depender de fonte)."""
    fluxo = f"BT /F1 14 Tf 40 200 Td ({texto}) Tj ET\n".encode("latin-1")
    objetos = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 420 260] "
        b"/Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
        b"<< /Length " + str(len(fluxo)).encode() + b" >>\nstream\n" + fluxo + b"endstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    ]
    saida = bytearray(b"%PDF-1.4\n")
    deslocamentos = []
    for numero, objeto in enumerate(objetos, start=1):
        deslocamentos.append(len(saida))
        saida += f"{numero} 0 obj\n".encode() + objeto + b"\nendobj\n"
    inicio = len(saida)
    total = len(objetos) + 1
    saida += f"xref\n0 {total}\n".encode() + b"0000000000 65535 f \n"
    for deslocamento in deslocamentos:
        saida += f"{deslocamento:010d} 00000 n \n".encode()
    saida += (
        f"trailer\n<< /Size {total} /Root 1 0 R >>\nstartxref\n{inicio}\n%%EOF\n".encode()
    )
    return bytes(saida)


def _com_ffmpeg(argumentos, sufixo):
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
    with tempfile.NamedTemporaryFile(suffix=sufixo) as saida:
        try:
            subprocess.run(
                ["ffmpeg", "-y", "-loglevel", "error", *argumentos, saida.name],
                check=True,
                timeout=60,
            )
        except (OSError, subprocess.SubprocessError):
            return None
        with open(saida.name, "rb") as arquivo:
            return arquivo.read()


def _ogg_de_teste():
    """OGG/Opus de um segundo (silêncio) via ffmpeg; sem ffmpeg, cabeçalho `OggS` mínimo.

    O `infer` reconhece o formato pelo "OggS" nos dois casos; com ffmpeg o Element também
    reproduz o áudio na sala (MX-49).
    """
    real = _com_ffmpeg(
        ["-f", "lavfi", "-i", "anullsrc=r=48000:cl=mono", "-t", "1", "-c:a", "libopus", "-f", "ogg"],
        ".ogg",
    )
    if real:
        return real
    cabecalho = (
        b"OggS"  # assinatura
        + b"\x00"  # versão
        + b"\x02"  # início de fluxo
        + b"\x00" * 8  # granule position
        + b"\x00\x00\x00\x01"  # serial do fluxo
        + b"\x00\x00\x00\x00"  # número de sequência
        + b"\x00\x00\x00\x00"  # checksum (o fixture não é decodificado)
        + b"\x00"  # nenhum segmento
    )
    return cabecalho + b"\x00" * 64


def _mp4_de_teste():
    """MP4 H.264 de um segundo via ffmpeg; sem ffmpeg, cabeçalho `ftyp` mínimo (MX-49)."""
    real = _com_ffmpeg(
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
    "image/png": ("imagem-do-cliente.png", _png_de_teste),
    "application/pdf": ("documento-do-cliente.pdf", _pdf_de_teste),
    "audio/ogg": ("audio-do-cliente.ogg", _ogg_de_teste),
    "video/mp4": ("video-do-cliente.mp4", _mp4_de_teste),
}


def _midia_do_evento(corpo):
    """Mimetype do evento que o Harness mandou em `{"message": dados}` (ou 404 se desconhecido)."""
    dados = corpo.get("message") or {}
    conteudo = dados.get("message") or {}
    for chave in ("imageMessage", "documentMessage"):
        bloco = conteudo.get(chave)
        if isinstance(bloco, dict):
            return (bloco.get("mimetype") or "").lower()
    return ""


class _Evo(BaseHTTPRequestHandler):
    """Evolution falsa: texto e mídia enviados, e o download da mídia que o cliente mandou."""

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
            codificado = corpo.get("media") or ""
            try:
                bytes_da_midia = base64.b64decode(codificado)
            except Exception:  # noqa: BLE001 - prova de ambiente, não de produto
                bytes_da_midia = b""
            # Só metadados: o conteúdo não fica em log (e o `media` em base64 é enorme).
            _registrar(
                self.diretorio,
                "evo_midia.jsonl",
                {
                    "instancia": self.path.rsplit("/", 1)[-1],
                    "number": corpo.get("number"),
                    "mediatype": corpo.get("mediatype"),
                    "mimetype": corpo.get("mimetype"),
                    "fileName": corpo.get("fileName"),
                    "caption": corpo.get("caption"),
                    "bytes": len(bytes_da_midia),
                    "sha256": hashlib.sha256(bytes_da_midia).hexdigest(),
                },
            )
            return self._json(201, {"key": {"id": "EVO-MIDIA"}, "status": "PENDING"})
        if "/chat/getBase64FromMediaMessage/" in self.path:
            mime = _midia_do_evento(corpo)
            fixture = FIXTURES.get(mime)
            if fixture is None:
                _registrar(
                    self.diretorio,
                    "evo_baixadas.jsonl",
                    {"mimetype": mime, "resultado": "nao_servido"},
                )
                return self._json(404, {"status": 404, "error": "Not Found", "message": "Message not found"})
            nome, gerar = fixture
            bytes_da_midia = gerar()
            _registrar(
                self.diretorio,
                "evo_baixadas.jsonl",
                {
                    "mimetype": mime,
                    "resultado": "servido",
                    "bytes": len(bytes_da_midia),
                    "sha256": hashlib.sha256(bytes_da_midia).hexdigest(),
                },
            )
            return self._json(
                200,
                {
                    "base64": base64.b64encode(bytes_da_midia).decode(),
                    "mimetype": mime,
                    "fileName": nome,
                },
            )
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


# ─────────────────────────── fase 2 (MX-43 a MX-46) ───────────────────────────


def _pedir_bruto(metodo, caminho, dados=None, token=None, content_type="application/octet-stream"):
    """Pedido com corpo cru (upload de mídia). Devolve status e corpo JSON (ou vazio)."""
    headers = {"Content-Type": content_type}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(BASE + caminho, data=dados, method=metodo, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            bruto = resp.read()
            return resp.status, (json.loads(bruto) if bruto else {})
    except urllib.error.HTTPError as e:
        bruto = e.read()
        try:
            corpo = json.loads(bruto or b"{}")
        except ValueError:
            corpo = {}
        return e.code, corpo


def webhook_midia(org, instancia, id_externo, tipo, legenda, push_name):
    """Cliente manda imagem ou documento pelo WhatsApp (MX-43).

    A mensagem leva só a referência (mimetype e legenda); o conteúdo o Harness baixa da
    Evolution falsa, que serve o fixture do tipo.
    """
    base = os.environ.get("HARNESS", "http://127.0.0.1:18010")
    telefone = os.environ.get("ACEITE_TELEFONE", "5521988887777")
    if tipo == "imagem":
        conteudo = {
            "imageMessage": {"mimetype": "image/png", "caption": legenda, "fileLength": 4096}
        }
        tipo_da_mensagem = "imageMessage"
    else:
        conteudo = {
            "documentMessage": {
                "mimetype": "application/pdf",
                "caption": legenda,
                "fileName": "documento-do-cliente.pdf",
                "fileLength": 2048,
            }
        }
        tipo_da_mensagem = "documentMessage"
    corpo = {
        "event": "messages.upsert",
        "instance": instancia,
        "apikey": "chave-falsa",
        "data": {
            "key": {
                "id": id_externo,
                "remoteJid": telefone + "@s.whatsapp.net",
                "fromMe": False,
            },
            "pushName": push_name,
            "messageType": tipo_da_mensagem,
            "message": conteudo,
            "messageTimestamp": int(time.time()),
        },
    }
    req = urllib.request.Request(
        f"{base}/webhook/evolution/{org}/{instancia}",
        data=json.dumps(corpo).encode(),
        method="POST",
        headers={
            "Content-Type": "application/json",
            "X-Evolution-Webhook-Secret": os.environ["ACEITE_SEGREDO_WEBHOOK"],
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        print(resp.status, json.loads(resp.read()))


# Chaves do protobuf que o Harness não abre (MX-45): o webhook as grava como texto de
# histórico, sem download. O conteúdo mínimo basta para o tipo ser reconhecido.
TIPOS_FORA_DO_ESCOPO = {
    "audio": ("audioMessage", {"ptt": True, "seconds": 4}),
    "video": ("videoMessage", {"seconds": 3}),
    "figurinha": ("stickerMessage", {"isAnimated": False}),
    "contato": ("contactMessage", {"displayName": "Contato de teste"}),
    "localizacao": ("locationMessage", {"degreesLatitude": -22.9, "degreesLongitude": -43.2}),
    "enquete": ("pollCreationMessage", {"name": "Enquete de teste", "selectableOptionsCount": 2}),
}


def escrever_fixtures(diretorio):
    """Grava o PNG e o PDF de teste (usados no envio do atendente, MX-44) e imprime os caminhos."""
    os.makedirs(diretorio, exist_ok=True)
    caminhos = {}
    for mime, (nome, gerar) in FIXTURES.items():
        caminho = os.path.join(diretorio, nome.replace("do-cliente", "do-atendente"))
        with open(caminho, "wb") as arquivo:
            arquivo.write(gerar())
        caminhos[mime] = caminho
    print(json.dumps(caminhos, ensure_ascii=False))


def webhook_tipo(org, instancia, id_externo, tipo, push_name):
    """Cliente manda um tipo que o Harness não abre (áudio, vídeo, figurinha, contato,
    localização, enquete): vira texto de histórico no turno e aviso na sala (MX-45)."""
    base = os.environ.get("HARNESS", "http://127.0.0.1:18010")
    telefone = os.environ.get("ACEITE_TELEFONE", "5521988887777")
    if tipo not in TIPOS_FORA_DO_ESCOPO:
        raise SystemExit(f"tipo fora do escopo desconhecido: {tipo}")
    chave, conteudo = TIPOS_FORA_DO_ESCOPO[tipo]
    corpo = {
        "event": "messages.upsert",
        "instance": instancia,
        "apikey": "chave-falsa",
        "data": {
            "key": {
                "id": id_externo,
                "remoteJid": telefone + "@s.whatsapp.net",
                "fromMe": False,
            },
            "pushName": push_name,
            "messageType": chave,
            "message": {chave: conteudo},
            "messageTimestamp": int(time.time()),
        },
    }
    req = urllib.request.Request(
        f"{base}/webhook/evolution/{org}/{instancia}",
        data=json.dumps(corpo).encode(),
        method="POST",
        headers={
            "Content-Type": "application/json",
            "X-Evolution-Webhook-Secret": os.environ["ACEITE_SEGREDO_WEBHOOK"],
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        print(resp.status, json.loads(resp.read()))


def webhook_aparelho(org, instancia, id_externo, texto):
    """Mensagem digitada no celular da loja (`fromMe`): MX-41/MX-42."""
    base = os.environ.get("HARNESS", "http://127.0.0.1:18010")
    telefone = os.environ.get("ACEITE_TELEFONE", "5521988887777")
    corpo = {
        "event": "messages.upsert",
        "instance": instancia,
        "apikey": "chave-falsa",
        "data": {
            "key": {
                "id": id_externo,
                "remoteJid": telefone + "@s.whatsapp.net",
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
        data=json.dumps(corpo).encode(),
        method="POST",
        headers={
            "Content-Type": "application/json",
            "X-Evolution-Webhook-Secret": os.environ["ACEITE_SEGREDO_WEBHOOK"],
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        print(resp.status, json.loads(resp.read()))


def atendente_envia_midia(sala, caminho, mime, nome, txn=None):
    """Atendente manda imagem, arquivo, áudio ou vídeo pela sala (o evento que o Element publica).

    O `msgtype` segue o tipo real, como o Element faz: `m.image`, `m.audio`, `m.video` e `m.file`
    para o resto (MX-44 e MX-49). O `txn` é opcional: repetir o mesmo valor reenvia a mesma
    transação do Element, que o Synapse deduplica (prova de reentrega do MX-49).
    """
    token = os.environ["ACEITE_TOKEN_HUMANO"]
    with open(caminho, "rb") as arquivo:
        dados = arquivo.read()
    status, d = _pedir_bruto(
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
    conteudo = {
        "msgtype": msgtype,
        "body": nome,
        "filename": nome,
        "url": mxc,
        "info": {"mimetype": mime, "size": len(dados)},
    }
    txn = txn or f"aceite-midia-{int(time.time() * 1000)}"
    status, d = _pedir(
        "PUT",
        f"/_matrix/client/v3/rooms/{_q(sala)}/send/m.room.message/{txn}",
        conteudo,
        token=token,
    )
    print(status, json.dumps(d, ensure_ascii=False))


def eventos(sala, limite=80):
    """Timeline da sala em forma de relatório: corpo, remetente, anexo e redação.

    Só o que é preciso para as asserções: nada de token, e o texto que aparece é o do
    roteiro (fixtures), nunca dado de cliente real.
    """
    token = os.environ["ACEITE_TOKEN_HUMANO"]
    status, d = _pedir(
        "GET",
        f"/_matrix/client/v3/rooms/{_q(sala)}/messages?dir=b&limit={int(limite)}",
        token=token,
    )
    if status != 200:
        print(json.dumps({"status": status, "erro": d}, ensure_ascii=False))
        return
    itens = []
    for evento in d.get("chunk", []):
        if evento.get("type") != "m.room.message":
            continue
        conteudo = evento.get("content") or {}
        arquivo = conteudo.get("file") if isinstance(conteudo.get("file"), dict) else {}
        bloco = {
            chave: valor
            for chave, valor in conteudo.items()
            if chave.startswith("com.") and chave.endswith(".attachment")
        }
        item = {
            "evento": evento.get("event_id"),
            "remetente": evento.get("sender"),
            "msgtype": conteudo.get("msgtype"),
            "corpo": conteudo.get("body"),
            "nome": conteudo.get("filename") or arquivo.get("filename"),
            "mxc": conteudo.get("url") or arquivo.get("url"),
            "bloco_de_anexo": bloco,
            "redigido": bool((evento.get("unsigned") or {}).get("redacted_because")),
        }
        redacao = (evento.get("unsigned") or {}).get("redacted_because") or {}
        if redacao:
            conteudo_da_redacao = redacao.get("content") or {}
            item["redigido_por"] = redacao.get("sender")
            item["bloco_expirado"] = {
                chave: valor
                for chave, valor in conteudo_da_redacao.items()
                if chave.endswith(".attachment_expired")
            }
        itens.append(item)
    print(json.dumps(itens, ensure_ascii=False, indent=2))


def evento(sala, event_id):
    token = os.environ["ACEITE_TOKEN_HUMANO"]
    status, d = _pedir(
        "GET",
        f"/_matrix/client/v3/rooms/{_q(sala)}/event/{_q(event_id)}",
        token=token,
    )
    print(json.dumps({"status": status, "corpo": d}, ensure_ascii=False, indent=2))


def midia_existe(mxc):
    """O arquivo responde no download de mídia do Synapse? (200 existe, 404 apagado)"""
    token = os.environ["ACEITE_TOKEN_HUMANO"]
    if not mxc.startswith("mxc://"):
        print(json.dumps({"erro": "endereco nao e mxc"}))
        return
    servidor, identificador = mxc[len("mxc://") :].split("/", 1)
    status, d = _pedir(
        "GET",
        f"/_matrix/client/v1/media/download/{_q(servidor)}/{_q(identificador)}",
        token=token,
    )
    print(json.dumps({"status": status, "erro": d.get("errcode")}, ensure_ascii=False))


def preparar_expiracao(banco, dias):
    """Recua o `sent_at` da mídia pendente, para o prazo vencer sem esperar dias (só no teste)."""
    import sqlite3

    corte = int(time.time() * 1000) - int(dias) * 86_400_000 - 60_000
    con = sqlite3.connect(banco)
    linhas = con.execute(
        "select count(*) from matrix_midia where redacted_at is null or deleted_at is null"
    ).fetchone()[0]
    con.execute(
        "update matrix_midia set sent_at=?, updated_at=? where redacted_at is null or deleted_at is null",
        (corte, int(time.time() * 1000)),
    )
    con.commit()
    print(json.dumps({"adiantadas": linhas, "sent_at": corte}))


def estado_expiracao(banco):
    import sqlite3

    con = sqlite3.connect(banco)
    try:
        linhas = con.execute(
            "select id, event_id, mxc, kind, name, sent_at, redacted_at, deleted_at,"
            " attempts, next_attempt_at, coalesce(last_error,'') from matrix_midia order by sent_at"
        ).fetchall()
    except sqlite3.OperationalError as erro:
        print(json.dumps({"erro": str(erro)}))
        return
    print(
        json.dumps(
            [
                {
                    "id": linha[0],
                    "evento": linha[1],
                    "mxc": linha[2],
                    "categoria": linha[3],
                    "nome": linha[4],
                    "sent_at": linha[5],
                    "redacted_at": linha[6],
                    "deleted_at": linha[7],
                    "tentativas": linha[8],
                    "proxima_tentativa": linha[9],
                    "ultimo_erro": linha[10],
                }
                for linha in linhas
            ],
            ensure_ascii=False,
            indent=2,
        )
    )


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
    elif comando == "webhook-midia":
        # <org> <instancia> <id> <imagem|documento> <legenda> <pushName>
        webhook_midia(argv[2], argv[3], argv[4], argv[5], argv[6], argv[7])
    elif comando == "webhook-aparelho":
        webhook_aparelho(argv[2], argv[3], argv[4], argv[5])
    elif comando == "webhook-tipo":
        # <org> <instancia> <id> <audio|video|figurinha|contato|localizacao|enquete> <pushName>
        webhook_tipo(argv[2], argv[3], argv[4], argv[5], argv[6])
    elif comando == "atendente-envia-midia":
        # <sala> <caminho> <mime> <nome> [txn]
        atendente_envia_midia(argv[2], argv[3], argv[4], argv[5], argv[6] if len(argv) > 6 else None)
    elif comando == "eventos":
        eventos(argv[2], argv[3] if len(argv) > 3 else 80)
    elif comando == "evento":
        evento(argv[2], argv[3])
    elif comando == "midia-existe":
        midia_existe(argv[2])
    elif comando == "preparar-expiracao":
        preparar_expiracao(argv[2], argv[3])
    elif comando == "estado-expiracao":
        estado_expiracao(argv[2])
    elif comando == "fixtures":
        escrever_fixtures(argv[2])
    else:
        raise SystemExit(f"comando desconhecido: {comando}")


if __name__ == "__main__":
    main(sys.argv)
