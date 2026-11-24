# Memory and throughput of the Matrix module (MX-30)

This document answers criterion **A13**: how much memory the Matrix module costs `poph serve` and how many messages
it mirrors per second. All the numbers here were measured in the development environment, with the release binary and
the real server; none is estimated. What could not be measured is marked as such, with the reason.

The measurement also found **a real defect in the room-creation path against a real Synapse** — the room is born
without a name, without a topic and without the configured power level. It is in the section "Defect found", with the
evidence; it was not fixed because the file belongs to another task.

## Summary

- **Memory with 1,000 rooms:** with the module on, it costs **+6.5 MB** over the server without the module (median of
  27.1 MB against 20.4 MB, across three rounds). The target proposed in the PRD — 20 MB with 1,000 rooms — is met with
  a margin of about three times.
- **Fixed cost:** with a single room, the module costs **+2.5 MB**. The rest grows slowly: **+3.7 MB** with 100 rooms,
  **+6.5 MB** with 1,000 and **+10.3 MB** with 10,000 — that is, ten times more rooms cost 1.6 times more memory than
  one thousand rooms. From 100 to 1,000 rooms it is about 3.1 kB per room; from 1,000 to 10,000, about 0.4 kB per room.
- **Throughput:** **739 messages/s** in the scenario of one message per room (with the creation of the 1,000 rooms
  included) and **2,648 to 2,811 messages/s** in the scenario of ten messages per room (amortized room creation). The
  bottleneck is the **fake Synapse**, which reaches 93% of one core; the Harness stays at 39–64%.
- **Development Synapse (20 rooms):** 4.8 messages/s. This number **does not measure mirroring throughput** — it
  measures the cost of creating a room on the development Synapse (about 5 rooms/s), and it is still contaminated by the
  defect below.

## Criterion A13

| What A13 asks for | What was measured | Where |
|---|---|---|
| RAM of `poph serve` with the module off | ~20.4 MB of RSS at steady state (20,272–20,984 kB), from 1 to 10,000 rooms | "Numbers: memory" |
| RAM with the module on, at 100, 1,000 and 10,000 rooms | +3.7 MB / +6.5 MB / +10.3 MB | "Numbers: memory" |
| Target of 20 MB with 1,000 rooms | measured at 6.5 MB; margin of about 3× | "Numbers: memory" |
| Mirrored messages per second | 739 msg/s (1 msg/room, 1,000 rooms) and 2,648–2,811 msg/s (10 msgs/room) | "Numbers: throughput" |
| Measurement against the real Synapse | 20 rooms, 5 rounds, 4.5–4.8 msg/s — contaminated number, see the section | "Development Synapse" |

## Method

Each round is an independent run:

1. **Seeding** (separate process, through the `poph-store` API, not by hand-written SQL): creates the org, the agent and
   N conversations with M customer messages each. With the module on, the org is switched on in the module **before**
   the messages, so everything that comes afterwards is mirrored.
2. **Real `poph serve`** (`target/release/poph`) on top of that database, with the default production configuration:
   concurrency 8, interval of 500 ms, log at `info` level. No accelerated loop.
3. **Fake Synapse in a separate process** (`scripts/medicao-memoria/synapse_falso.py`, standard library only), so that
   its consumption does not enter the RSS of `poph serve` and so that the 10,000-room scale fits in the time available.
   It answers the same routes as the fake used in the module's tests: ghost registration, profile, `createRoom` with
   alias, invite, join, members, state, send with `txnId` deduplication, and the probe `/_matrix/client/versions`.
4. **RSS read from the process** (`/proc/<pid>/status`: `VmRSS` and the peak `VmHWM`), sampled once per second for 10 s
   **after** the queue empties. The number in the table is the median of those 10 samples; the peak is `VmHWM`. CPU
   comes from the ticks in `/proc/<pid>/stat` divided by the wall-clock time.
5. **Throughput** = unique sends counted by the fake (deduplicated by `txnId`) divided by the time between the server
   answering on `/healthz` and the last message reaching the fake.

Rounds: three per scenario at 1, 100, 1,000 and 10,000 rooms; two in the throughput scenario; five in the development
Synapse. Commands at the end of the document.

### Declared deviations from production

- **Fake Synapse, not the real Synapse**, for the 100 to 10,000 room scale: the development Synapse creates about 5
  rooms per second, so 10,000 rooms would take about half an hour per round, in an environment that other agents use.
  The real round was done at a small scale (20 rooms) and is reported separately.
- **Fake and server on the same machine**: the 93% CPU of the fake and the contention between the two processes make
  the measured throughput a **floor**, not a ceiling.
- **Release binary**, with no `perf` and no allocator instrumentation: the number is the total RSS of the process,
  including the SQLite page cache and the allocator — it is not the "memory of the module" in isolation.

## Numbers: memory

RSS at steady state (median of 10 samples per round; kB), `poph serve` release build:

| Rooms | Module off (3 rounds) | Module on (3 rounds) | Increase |
|---|---|---|---|
| 1 | 20,152 (20,084–20,468) | 22,720 (22,664–22,776) | **+2.5 MB** |
| 100 | 20,768 (20,624–20,788) | 24,526 (24,272–24,660) | **+3.7 MB** |
| 1,000 | 20,440 (20,272–20,984) | 27,092 (26,780–27,152) | **+6.5 MB** |
| 10,000 | 20,596 (20,404–20,628) | 31,186 (31,024–31,348, 2 rounds) | **+10.3 MB** |

Peak (`VmHWM`) in the same rounds: 20,768 kB (off, 100 rooms) to 31,348 kB (on, 10,000 rooms). The peak was between 0 and
24 kB above steady state in every round — the module has no allocation spikes in the normal cycle; what exists is the
plateau.

Reading:

- **Fixed cost of 2.5 MB** (one room): HTTP client, ghost ids, the connection probe, the three loops, the metrics
  registry and the first database accesses.
- **Sublinear growth**: 100 → 1,000 rooms adds 2.8 MB (~3.1 kB per room); 1,000 → 10,000 adds 3.8 MB (~0.4 kB per
  room). This is the expected behavior of a capped cache (the ghost caches are capped at 1,024 entries per process)
  added to what the database keeps in memory for the cursor and for incremental reading.
- **I did not measure the split by component** (how much is the module's cache, how much is SQLite pages): the project
  has no allocator instrumentation, and creating it is outside the scope of this task. What is measured is the total,
  which is the criterion's number.

## Numbers: throughput

| Scenario | Messages | Drain time | Throughput | Serve CPU | Fake CPU |
|---|---|---|---|---|---|
| On, 100 rooms, 1 msg/room | 100 | 0.25 s (0.25–0.25) | 403.2 msg/s (403–407) | 8% | 8–16% |
| On, 1,000 rooms, 1 msg/room | 1,000 | 1.35 s (1.35–1.36) | **739.1 msg/s** (737.5–740.7) | 61% | 92–95% |
| On, 10,000 rooms, 1 msg/room | 10,000 | 14.66 s (14.65–14.66) | **682.4 msg/s** (682.0–682.7) | 64% | 93–94% |
| On, 1,000 rooms, 10 msgs/room | 10,000 | 3.67 s (3.56–3.78) | **2,729 msg/s** (2,648–2,811) | 39% | 70–73% |

How to read it:

- In the first three scenarios, **most of the time is room creation**, not message sending: N rooms are created, the
  customer's ghost is invited, joins, and metadata is set. Room creation runs at ~700–740 rooms/s against the fake.
- Only the 10-messages-per-room scenario separates sending from creation: 10,000 messages in 3.67 s. From it comes the
  reading that **sending a message by itself is faster than the number in the table**: discounting the ~1.35 s that the
  creation of 1,000 rooms cost in the previous scenario, about 2.3 s remain for 10,000 messages (≈4,300 msg/s). This
  number is **derived**, not measured directly — a direct measurement would require seeding the rooms and only then
  injecting messages, and the current script does both in the same seeding.
- **The fake is the bottleneck** from 1,000 rooms on: 92–95% of one core against 61–64% for the Harness. The measured
  throughput is therefore a floor for what the module sustains.
- At the 10,000-room scale throughput drops 8% compared with 1,000 (739 → 682), with the same Harness CPU: this is the
  cost of keeping 10,000 rooms in the database and in the loop, not a degradation of sending.

## Noise

- **Between rounds of the same scenario** (what matters most for comparing with/without the module): the RSS without
  the module varied by 712 kB (20,272–20,984 kB); with the module, by 372 kB at 1,000 rooms. The measured increase
  (6.5 MB) is **nine times** that variation.
- **In throughput:** the range between rounds was 3.2 msg/s at 1,000 rooms (0.4% of the median) and 162 msg/s in the
  10-messages-per-room scenario (5.9% of the median).
- **In drain time:** ±0.11 s at 10 messages per room; ±0.01 s in the one-message scenarios.
- **Runs:** the main script ran once in full (three rounds in each scenario at 100, 1,000 and 10,000 rooms, two in the
  throughput one). After that, the one-room scenario (fixed cost) and the real mode were run in separate runs — the real
  one five times in total, with the results within the range of 4.5–4.8 msg/s.

## Total duration

The full script (100/1,000/10,000 rooms, off and on, throughput and real) took **7 min 27 s** of wall-clock time
(03:10:24 to 03:17:51 UTC, measured from the timestamps in the server logs):

| Scenario | Seeding | Drain | Sampling | Total per round |
|---|---|---|---|---|
| Off, 100–1,000 rooms | 0.2–0.7 s | — | 10 s | ~11 s |
| Off, 10,000 rooms | 4.6–4.8 s | — | 10 s | ~15–16 s |
| On, 100–1,000 rooms, 1 msg | 0.2–0.7 s | 0.02–1.35 s | 10 s | ~11–12 s |
| On, 10,000 rooms | 4.8 s | 14.66 s | 10 s | ~30 s |
| On, 1,000 rooms, 10 msgs | 2.0–2.1 s | 3.56–3.78 s | 10 s | ~16 s |
| Real, 20 rooms | 0.2 s | 4.18–4.40 s | 10 s | ~15 s |

Most of each round is the fixed 10 s RSS sampling window, not the work.

## Development Synapse (MX-01, port 18008)

Five rounds of 20 rooms and 20 messages:

| Round | RSS at startup | RSS at steady state | Drain | Throughput |
|---|---|---|---|---|
| 1 | 23,464 kB | 24,056 kB | 4.40 s | 4.5 msg/s |
| 2 | 23,672 kB | 24,116 kB | 4.19 s | 4.8 msg/s |
| 3 | 23,180 kB | 23,644 kB | 4.19 s | 4.8 msg/s |
| 4 | 22,852 kB | 23,316 kB | 4.19 s | 4.8 msg/s |
| 5 | 23,120 kB | 23,644 kB | 4.18 s | 4.8 msg/s |

**These numbers do not measure the module's throughput.** In all five rounds, the 20 rooms failed on the first attempt
and only entered on a later pass of the loop; the 4.2 s time is the cost of creating a room on the development Synapse
**plus** one extra pass of the loop. The memory of the real round (23.3–24.1 MB at steady state) is consistent with the
local rounds at the same scale, but the environment is shared with other agents, so it does not serve as a reference
measurement. The cause of the failure is below.

## Defect found (not fixed)

**The room created against a real Synapse is born without a name, without a topic and without the configured power
level.**

What happens: the `createRoom` sent by the module includes `power_level_content_override` with the agent's ghost at the
level of `matrix.poder_agente`, whose default is **50** (`crates/poph-types/src/config.rs`, `poder_agente: 50`). On the
real Synapse, whoever creates the room needs level **100** to write its own `m.room.power_levels`: with 50, the request
returns **403** — after the room and the alias already exist. The next pass finds the alias (`M_ROOM_IN_USE` → resolves
the alias → continues), so the Harness "recovers", but the room **ends up without `m.room.name` and without
`m.room.topic`**, and without the power level the module wanted to apply.

Evidence (captured by a diagnostic proxy between `poph serve` and the development Synapse, which records method, path,
status and `errcode` — no token):

```
POST /_matrix/client/v3/createRoom user=@ph_ag_… status=403 errcode=M_FORBIDDEN
  erro=You don't have permission to post that to the room. user_level (50) < send_level (100)
POST /_matrix/client/v3/createRoom user=@ph_ag_… status=400 errcode=M_ROOM_IN_USE
  erro=Room alias already taken
```

State of the room created in that round, read through the Matrix API as the agent's ghost (who is the creator):
`m.room.name` → `404 M_NOT_FOUND`, `m.room.topic` → `404`, `m.room.power_levels` → `404`, while
`com.sofacoding.atendimento.metadata` is present (200). The room exists, both members are inside, and it is **without a
name**.

Control test, with the same body and only the power level changed:

| `power_level_content_override` of the creator | `createRoom` | `m.room.name` afterwards |
|---|---|---|
| 50 | **403** `M_FORBIDDEN` (`user_level (50) < send_level (100)`) | `404 M_NOT_FOUND` |
| 100 | **200** | `{"name":"Nome Visivel"}` |

Product impact: this is criterion **A1** ("correct customer and agent names" in the room) failing on the real Synapse —
the room shows up in Element with the technical identifier, not the customer's name. Also, each conversation pays one
doomed call and one extra loop pass before the room becomes usable. The local tests do not catch this because the fake
Synapse does not validate power levels.

Not fixed: the file belongs to MX-11 and this task's rule is to stop and return with the evidence. The likely fix is not
to lower the **creator** below 100 and to apply the agent ghost's power level only after creation (or to use
`events`/`users_default` instead of overriding the creator). A test against the real Synapse is also worth doing, since
the fake does not reproduce it.

### How to reproduce the defect (against the development Synapse, without the Harness)

With the `$TOKEN` of the Harness's Application Service and `$BASE` of the Synapse (`http://127.0.0.1:18008`):

1. `POST /_matrix/client/v3/register?kind=user` with
   `{"type":"m.login.application_service","username":"ph_ag_pl50","inhibit_login":true}`.
2. Create the room **with the power level the module sends today**, as the newly registered ghost:
   `POST /_matrix/client/v3/createRoom?user_id=@ph_ag_pl50:matrix-dev.local` with
   `{"room_alias_name":"ph_c_pl50","name":"Nome Visivel","topic":"Atendimento",
   "creation_content":{"type":"com.sofacoding.atendimento"},
   "power_level_content_override":{"users":{"@ph_ag_pl50:matrix-dev.local":50}},
   "preset":"private_chat"}` → **403** `M_FORBIDDEN` (`user_level (50) < send_level (100)`).
3. Repeat with `100` instead of `50` → **200**, and
   `GET /_matrix/client/v3/rooms/{room_id}/state/m.room.name?user_id=@ph_ag_pl50:matrix-dev.local`
   returns `{"name":"Nome Visivel"}`. With 50, the same read returns `404 M_NOT_FOUND` — the room exists and has no name.

The same happens through the Harness path: start `poph serve` with `[matrix]` pointing to the development Synapse and the
database of a real round; the log shows the `motivo=sala_rejeitada` (reason=room_rejected) failures on the first pass and
mirroring completes on the next one, with the rooms lacking `m.room.name`.

## What was NOT measured

- **RAM with the real Synapse at scale**: 10,000 rooms at ~5 rooms/s would take about half an hour per round, in an
  environment that other agents use. The 10,000-room number comes from the fake Synapse.
- **Throughput of sending in isolation from room creation**: it would require seeding the rooms, waiting for the module
  to stabilize and only then injecting messages. The current script seeds both together; the ~4,300 msg/s figure in the
  reading is derived, not measured.
- **The split of the memory increase by component** (module cache × SQLite pages × allocator): no allocator
  instrumentation.
- **Behavior after hours of operation** (fragmentation, WAL growth): the longest round was 30 s.
- **The visual state in Element** (A1 for real, with the name on screen): it is MX-31, owned by someone else.
- **Two Harness instances competing for the same machine**: phase 3.

## Known limits

- **RSS is for the whole process**, not the module: it includes the SQLite page cache and what the allocator does not
  return to the system. It is useful for comparing with/without the module on the same machine, not as an absolute
  promise on other hardware.
- **The fake is faster than any real Synapse**; the 739–2,811 msg/s throughput is what the module does when the other
  side keeps up, and the real ceiling will be set by Synapse.
- **The real round was contaminated** by the power-level defect (one extra loop pass on every room), so 4.8 msg/s is a
  pessimistic floor, not a clean measurement.
- **The 10 s sampling window** captures the steady state, not the peak of a burst; `VmHWM` covers the allocation peak and
  was the same as steady state.
- **The same SSD and the same CPU** host the server, the fake and the database: the absolute throughput varies with the
  machine's load (the five real rounds show the variation).
- **The script does not delete the databases** in `target/mx30/`: they are disposable and stay out of git.

## How to reproduce

```bash
cd apps/pop-harness
export PATH="$HOME/.cargo/bin:$PATH"
cargo build --release --locked -p poph          # the script uses target/release/poph

bash scripts/medir-memoria.sh --modo desligado --salas 1000 --rodada 1
bash scripts/medir-memoria.sh --modo ligado   --salas 1000 --mensagens 1  --rodada 1
bash scripts/medir-memoria.sh --modo ligado   --salas 1000 --mensagens 10 --rodada 1   # throughput
bash scripts/medir-memoria.sh --modo real     --salas 20   --rodada 1                  # development Synapse

bash scripts/medir-memoria-roteiro.sh           # the full matrix (7–8 min)
bash scripts/medir-memoria-roteiro.sh rapido    # 100 rooms, quick check
```

Each round writes one line to `target/mx30/memoria.csv` (outside git), with startup RSS, steady state, peak, drain time,
throughput, CPU of both processes, rooms created, unique sends, invites, requests and seeding time. The logs of each
round are in `target/mx30/logs/`.

The `real` mode reads the credentials of the development Synapse from `~/.cache/poph-matrix/credenciais.env`
(generated by `scripts/matrix-dev.sh`) and does not print any token: the tokens go through environment variables and
never onto the command line or into the CSV.
