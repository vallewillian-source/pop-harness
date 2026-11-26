#!/usr/bin/env python3
"""Fast fake Synapse for the Matrix memory and throughput measurement (MX-30).

Runs in its own process, so its memory does not enter the RSS of `poph serve`.
It answers the same routes as the fake used by the module's tests (`crates/poph-server/tests/matrix_inicio.rs`):
user registration, profile and display name, room creation with alias, invite, join, members, state, send with
`txnId` deduplication, and the `/_matrix/client/versions` probe.

It keeps nothing on disk. The counters come out at `GET /_fake/estatisticas`, which needs no token.

Usage:
  MX30_AS_TOKEN=... python3 fake_synapse.py --porta 18431 [--latencia-ms 0]

The AS token is read from the environment (never from the command line, so it does not show up in `ps`).
Without the right token, the fake answers 401, as Synapse does.

Standard library only. `-I` avoids loading modules from the current directory.
"""

import argparse
import json
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote, urlparse, parse_qs

SERVER = "matrix-dev.local"
API = "/_matrix/client/v3"


class State:
    def __init__(self, token, latency_ms):
        self.token = token
        self.latency = latency_ms / 1000.0
        self.lock = threading.Lock()
        self.users = {}
        self.rooms = {}
        self.aliases = {}
        self.next_room = 0
        self.requests = 0
        self.sends = 0
        self.unique_sends = 0
        self.invites = 0
        self.rooms_created = 0
        self.started = time.time()


STATE = None


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "synapse-falso-mx30"
    disable_nagle_algorithm = True

    def log_message(self, *args):
        return

    def _send(self, status, body):
        data = json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def _read_body(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        if not raw:
            return {}
        try:
            return json.loads(raw.decode("utf-8"))
        except ValueError:
            return {}

    def _handle(self, method):
        url = urlparse(self.path)
        path = url.path
        query = parse_qs(url.query)

        if path == "/_fake/estatisticas" and method == "GET":
            with STATE.lock:
                data = {
                    "requisicoes": STATE.requests,
                    "envios": STATE.sends,
                    "envios_unicos": STATE.unique_sends,
                    "salas": len(STATE.rooms),
                    "usuarios": len(STATE.users),
                    "convites": STATE.invites,
                    "segundos_no_ar": round(time.time() - STATE.started, 3),
                }
            return self._send(200, data)

        body = self._read_body()
        if self.headers.get("Authorization") != f"Bearer {STATE.token}":
            if path != "/_matrix/client/versions":
                return self._send(401, {"errcode": "M_UNKNOWN_TOKEN", "error": "Invalid token"})

        if STATE.latency:
            time.sleep(STATE.latency)

        with STATE.lock:
            STATE.requests += 1
            status, out = self._route(method, path, query, body)
        self._send(status, out)

    def _route(self, method, path, query, body):
        if path == "/_matrix/client/versions":
            return 200, {"versions": ["v1.11"]}

        if not path.startswith(API):
            return 404, {"errcode": "M_UNRECOGNIZED", "error": "Route does not exist in the fake"}
        parts = [unquote(p) for p in path[len(API):].split("/") if p]
        who = (query.get("user_id") or [""])[0]

        if method == "POST" and parts == ["register"]:
            localpart = body.get("username", "")
            user = f"@{localpart}:{SERVER}"
            if user in STATE.users:
                return 400, {"errcode": "M_USER_IN_USE", "error": "User ID already taken."}
            STATE.users[user] = None
            return 200, {"user_id": user}

        if len(parts) == 2 and parts[0] == "profile" and method == "GET":
            if parts[1] not in STATE.users:
                return 404, {"errcode": "M_NOT_FOUND", "error": "User does not exist"}
            name = STATE.users[parts[1]]
            return 200, ({"displayname": name} if name else {})

        if len(parts) == 3 and parts[0] == "profile" and parts[2] == "displayname" and method == "PUT":
            STATE.users[parts[1]] = body.get("displayname")
            return 200, {}

        if parts == ["createRoom"] and method == "POST":
            alias = None
            if body.get("room_alias_name"):
                alias = f"#{body['room_alias_name']}:{SERVER}"
                if alias in STATE.aliases:
                    return 400, {"errcode": "M_ROOM_IN_USE", "error": "Room alias already taken."}
            STATE.next_room += 1
            room = f"!{STATE.next_room}:{SERVER}"
            members = {who} if who else set()
            STATE.rooms[room] = {
                "members": set(members),
                "invited": set(),
                "state": {},
                "txns": set(),
            }
            STATE.rooms_created += 1
            if alias:
                STATE.aliases[alias] = room
            return 200, {"room_id": room}

        if len(parts) == 3 and parts[0] == "directory" and parts[1] == "room" and method == "GET":
            room = STATE.aliases.get(parts[2])
            if room is None:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Alias not found"}
            return 200, {"room_id": room}

        if len(parts) == 3 and parts[0] == "rooms" and parts[2] == "invite" and method == "POST":
            room = STATE.rooms.get(parts[1])
            if room is None:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Room does not exist"}
            room["invited"].add(body.get("user_id", ""))
            STATE.invites += 1
            return 200, {}

        if len(parts) == 2 and parts[0] == "join" and method == "POST":
            target = parts[1]
            room_id = STATE.aliases.get(target) if target.startswith("#") else target
            room = STATE.rooms.get(room_id) if room_id else None
            if room is None:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Room does not exist"}
            room["members"].add(who)
            return 200, {"room_id": room_id}

        if len(parts) == 3 and parts[0] == "rooms" and parts[2] == "joined_members" and method == "GET":
            room = STATE.rooms.get(parts[1])
            if room is None:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Room does not exist"}
            return 200, {"joined": {m: {} for m in room["members"]}}

        if len(parts) >= 4 and parts[0] == "rooms" and parts[2] == "state":
            room = STATE.rooms.get(parts[1])
            if room is None:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Room does not exist"}
            key = (parts[3], "/".join(parts[4:]))
            if method == "PUT":
                room["state"][key] = body
                return 200, {"event_id": "$estado"}
            if key in room["state"]:
                return 200, room["state"][key]
            return 404, {"errcode": "M_NOT_FOUND", "error": "Event does not exist"}

        if len(parts) == 5 and parts[0] == "rooms" and parts[2] == "send" and method == "PUT":
            room = STATE.rooms.get(parts[1])
            if room is None:
                return 404, {"errcode": "M_NOT_FOUND", "error": "Room does not exist"}
            STATE.sends += 1
            txn = parts[4]
            if txn not in room["txns"]:
                room["txns"].add(txn)
                STATE.unique_sends += 1
            return 200, {"event_id": f"$msg_{txn}"}

        return 404, {"errcode": "M_UNRECOGNIZED", "error": "Route does not exist in the fake"}

    def do_GET(self):
        self._handle("GET")

    def do_POST(self):
        self._handle("POST")

    def do_PUT(self):
        self._handle("PUT")


def main():
    global STATE
    parser = argparse.ArgumentParser()
    parser.add_argument("--porta", dest="port", type=int, required=True)
    parser.add_argument("--latencia-ms", dest="latency_ms", type=int, default=0)
    parser.add_argument("--endereco", dest="address", default="127.0.0.1")
    args = parser.parse_args()
    token = os.environ.get("MX30_AS_TOKEN", "")
    if not token:
        raise SystemExit("MX30_AS_TOKEN missing from the environment")
    STATE = State(token, args.latency_ms)
    server = ThreadingHTTPServer((args.address, args.port), Handler)
    server.daemon_threads = True
    print(f"fake synapse on {args.address}:{args.port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
