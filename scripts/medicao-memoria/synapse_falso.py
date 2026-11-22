#!/usr/bin/env python3
"""Synapse falso e rápido para a medição de RAM e vazão do módulo Matrix (MX-30).

Roda em processo próprio, para que o consumo de memória dele não entre no RSS do `poph serve`.
Responde às mesmas rotas que o falso dos testes do módulo (`crates/poph-server/tests/matrix_inicio.rs`):
registro de usuário virtual, perfil e nome de exibição, criação de sala com alias, convite, entrada,
membros, estado, envio com deduplicação por `txnId` e a sonda `/_matrix/client/versions`.

Não guarda nada em disco. Os contadores saem em `GET /_fake/estatisticas`, que não exige token.

Uso:
  MX30_AS_TOKEN=... python3 synapse_falso.py --porta 18431 [--latencia-ms 0]

O token do AS é lido do ambiente (nunca da linha de comando, para não aparecer em `ps`). Sem o
token certo, o falso responde 401, como o Synapse.

Só biblioteca padrão. `-I` evita carregar módulos do diretório atual.
"""

import argparse
import json
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote, urlparse, parse_qs

SERVIDOR = "matrix-dev.local"
API = "/_matrix/client/v3"


class Estado:
    def __init__(self, token, latencia_ms):
        self.token = token
        self.latencia = latencia_ms / 1000.0
        self.trava = threading.Lock()
        self.usuarios = {}  # id -> displayname | None
        self.salas = {}  # room_id -> dict(membros=set, convidados=set, estado=dict, txns=set)
        self.aliases = {}  # alias -> room_id
        self.proxima_sala = 0
        self.requisicoes = 0
        self.envios = 0  # chamadas de send, inclusive repetidas
        self.envios_unicos = 0  # (sala, txnId) distintos: o que de fato entrou na sala
        self.convites = 0
        self.criacoes_de_sala = 0
        self.inicio = time.time()


ESTADO = None


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "synapse-falso-mx30"
    # Sem isto, cabeçalho e corpo saem em dois envios e o Nagle + ACK atrasado somam ~40 ms por
    # resposta no localhost. Foi o que limitou o falso a ~21 salas/s no primeiro diagnóstico da MX-30.
    disable_nagle_algorithm = True

    def log_message(self, *args):  # silêncio: o log não entra na medição
        return

    def _enviar(self, status, corpo):
        dados = json.dumps(corpo).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(dados)))
        self.end_headers()
        self.wfile.write(dados)

    def _ler_corpo(self):
        tamanho = int(self.headers.get("Content-Length") or 0)
        bruto = self.rfile.read(tamanho) if tamanho else b""
        if not bruto:
            return {}
        try:
            return json.loads(bruto.decode("utf-8"))
        except ValueError:
            return {}

    def _tratar(self, metodo):
        url = urlparse(self.path)
        caminho = url.path
        consulta = parse_qs(url.query)

        if caminho == "/_fake/estatisticas" and metodo == "GET":
            with ESTADO.trava:
                dados = {
                    "requisicoes": ESTADO.requisicoes,
                    "envios": ESTADO.envios,
                    "envios_unicos": ESTADO.envios_unicos,
                    "salas": len(ESTADO.salas),
                    "usuarios": len(ESTADO.usuarios),
                    "convites": ESTADO.convites,
                    "segundos_no_ar": round(time.time() - ESTADO.inicio, 3),
                }
            return self._enviar(200, dados)

        corpo = self._ler_corpo()
        if self.headers.get("Authorization") != f"Bearer {ESTADO.token}":
            if caminho != "/_matrix/client/versions":
                return self._enviar(401, {"errcode": "M_UNKNOWN_TOKEN", "error": "Token inválido"})

        if ESTADO.latencia:
            time.sleep(ESTADO.latencia)

        with ESTADO.trava:
            ESTADO.requisicoes += 1
            status, saida = self._rotear(metodo, caminho, consulta, corpo)
        self._enviar(status, saida)

    def _rotear(self, metodo, caminho, consulta, corpo):
        if caminho == "/_matrix/client/versions":
            return 200, {"versions": ["v1.11"]}

        if not caminho.startswith(API):
            return 404, {"errcode": "M_UNRECOGNIZED", "error": "Rota não existe no falso"}
        partes = [unquote(p) for p in caminho[len(API):].split("/") if p]
        quem = (consulta.get("user_id") or [""])[0]

        if metodo == "POST" and partes == ["register"]:
            localpart = corpo.get("username", "")
            usuario = f"@{localpart}:{SERVIDOR}"
            if usuario in ESTADO.usuarios:
                return 400, {"errcode": "M_USER_IN_USE", "error": "User ID already taken."}
            ESTADO.usuarios[usuario] = None
            return 200, {"user_id": usuario}

        if len(partes) == 2 and partes[0] == "profile" and metodo == "GET":
            if partes[1] not in ESTADO.usuarios:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Usuário não existe"}
            nome = ESTADO.usuarios[partes[1]]
            return 200, ({"displayname": nome} if nome else {})

        if len(partes) == 3 and partes[0] == "profile" and partes[2] == "displayname" and metodo == "PUT":
            ESTADO.usuarios[partes[1]] = corpo.get("displayname")
            return 200, {}

        if partes == ["createRoom"] and metodo == "POST":
            alias = None
            if corpo.get("room_alias_name"):
                alias = f"#{corpo['room_alias_name']}:{SERVIDOR}"
                if alias in ESTADO.aliases:
                    return 400, {"errcode": "M_ROOM_IN_USE", "error": "Room alias already taken."}
            ESTADO.proxima_sala += 1
            sala = f"!{ESTADO.proxima_sala}:{SERVIDOR}"
            membros = {quem} if quem else set()
            ESTADO.salas[sala] = {
                "membros": set(membros),
                "convidados": set(),
                "estado": {},
                "txns": set(),
            }
            ESTADO.criacoes_de_sala += 1
            if alias:
                ESTADO.aliases[alias] = sala
            return 200, {"room_id": sala}

        if len(partes) == 3 and partes[0] == "directory" and partes[1] == "room" and metodo == "GET":
            sala = ESTADO.aliases.get(partes[2])
            if sala is None:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Alias não encontrado"}
            return 200, {"room_id": sala}

        if len(partes) == 3 and partes[0] == "rooms" and partes[2] == "invite" and metodo == "POST":
            sala = ESTADO.salas.get(partes[1])
            if sala is None:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Sala não existe"}
            sala["convidados"].add(corpo.get("user_id", ""))
            ESTADO.convites += 1
            return 200, {}

        if len(partes) == 2 and partes[0] == "join" and metodo == "POST":
            alvo = partes[1]
            sala_id = ESTADO.aliases.get(alvo) if alvo.startswith("#") else alvo
            sala = ESTADO.salas.get(sala_id) if sala_id else None
            if sala is None:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Sala não existe"}
            sala["membros"].add(quem)
            return 200, {"room_id": sala_id}

        if len(partes) == 3 and partes[0] == "rooms" and partes[2] == "joined_members" and metodo == "GET":
            sala = ESTADO.salas.get(partes[1])
            if sala is None:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Sala não existe"}
            return 200, {"joined": {m: {} for m in sala["membros"]}}

        if len(partes) >= 4 and partes[0] == "rooms" and partes[2] == "state":
            sala = ESTADO.salas.get(partes[1])
            if sala is None:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Sala não existe"}
            chave = (partes[3], "/".join(partes[4:]))
            if metodo == "PUT":
                sala["estado"][chave] = corpo
                return 200, {"event_id": "$estado"}
            if chave in sala["estado"]:
                return 200, sala["estado"][chave]
            return 404, {"errcode": "M_NOT_FOUND", "error": "Evento não existe"}

        if len(partes) == 5 and partes[0] == "rooms" and partes[2] == "send" and metodo == "PUT":
            sala = ESTADO.salas.get(partes[1])
            if sala is None:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Sala não existe"}
            ESTADO.envios += 1
            txn = partes[4]
            if txn not in sala["txns"]:
                sala["txns"].add(txn)
                ESTADO.envios_unicos += 1
            return 200, {"event_id": f"$msg_{txn}"}

        return 404, {"errcode": "M_UNRECOGNIZED", "error": "Rota não existe no falso"}

    def do_GET(self):
        self._tratar("GET")

    def do_POST(self):
        self._tratar("POST")

    def do_PUT(self):
        self._tratar("PUT")


def main():
    global ESTADO
    parser = argparse.ArgumentParser()
    parser.add_argument("--porta", type=int, required=True)
    parser.add_argument("--latencia-ms", type=int, default=0)
    parser.add_argument("--endereco", default="127.0.0.1")
    args = parser.parse_args()
    token = os.environ.get("MX30_AS_TOKEN", "")
    if not token:
        raise SystemExit("MX30_AS_TOKEN ausente no ambiente")
    ESTADO = Estado(token, args.latencia_ms)
    servidor = ThreadingHTTPServer((args.endereco, args.porta), Handler)
    servidor.daemon_threads = True
    print(f"synapse falso em {args.endereco}:{args.porta}", flush=True)
    servidor.serve_forever()


if __name__ == "__main__":
    main()
