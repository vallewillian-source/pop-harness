# Console Visual Baseline (RF-08)

This directory holds the visual baseline for the primary screens of the POP Harness Console, captured prior to the refactoring for internationalization (English codebase and comment removal).

The purpose of this baseline is to serve as a comparative visual reference ensuring exact parity of layout, typography, spacing, and components during and after frontend refactors and renamings.

> **Important — Provisional Baseline:**
> The current set of captures in `design_assets/20261123-refactor-baseline/` is **provisional**. The Console layout is actively evolving (tasks CF-09 in progress and CF-10 pending). The definitive baseline capture for CI gating will be generated after the completion and acceptance of the final layout (CF-10).

---

## 1. Captured Screens

All captures were generated in PNG format along with their respective DOM HTML snapshots:

| File | Screen / Route | Captured State |
|---|---|---|
| `01-login.png` | Login (`/login`) | Clean initial state: admin token field, in-memory usage guidance, and submit button. |
| `02-orgs-lista.png` | Organizations (`/orgs`) | Organization table with demo seed data, filter tabs ("All", "Active", "Inactive"), global search, and cluster status. |
| `03-orgs-inspetor.png` | Organization Inspector (`/orgs`) | Side inspector drawer opened over the selected organization, displaying metadata, active agent count, node status, and actions. |
| `04-agente-instrucoes.png` | Agent / Instructions (`/orgs/:id/agents/:id/instructions`) | Sub-navigation with the 7 agent tabs, structured form for identity/tone/scope, and compiled Markdown prompt preview. |
| `05-agente-base-conhecimento.png` | Agent / Knowledge Base (`/orgs/:id/agents/:id/knowledge-base`) | Hybrid search view and table of knowledge groups linked to the agent. |
| `06-chat-teste.png` | Test Chat (`/orgs/:id/agents/:id/test-chat`) | Active test session with message exchange (user and agent), status indicator, and message composer. |
| `07-operacao.png` | Operation & Health (`/orgs/:id/operacao`) | Operational telemetry dashboard with engine health, runtime metrics, declared plugins, and cluster card. |

---

## 2. Resolution and Viewport

Captures follow the reference desktop layout specification (`console/UX_SPEC.md`):

- **Resolution / Viewport:** `1440 × 900` pixels
- **Device Scale Factor (DPR):** `1.0`
- **Theme:** Dark (canonical background `#151719` / `--pop-bg`)
- **Browser:** Chromium / Google Chrome Headless with hardware acceleration disabled for maximum rendering reproducibility.

---

## 3. Database and Demo Seed Data

The environment uses an isolated temporary SQLite database provisioned from official migrations (`poph migrate`).

- **Isolation and Security:** Zero real customer data, zero production accounts, and zero private credentials. POP Harness is a public and auditable project.
- **Fictional Entities:**
  - Demo organizations: `Acme Demonstração` (slug: `acme`) and `Beta Serviços` (slug: `beta`).
  - Department: `Suporte` (General Support).
  - Agent: `Bia Atendente` (slug: `bia-atendente`, model `gemini-3.8-flash-tiered`).
  - Knowledge group and items: `Políticas e Horários` with operational items.
  - Test conversation: Session with predefined question-and-answer messages.
- **Temporal Determinism:**
  - All records use fixed timestamps (`1795484000000`, corresponding to 2026-11-23 at 22:33:20 UTC-03:00), eliminating date differences in listings and inspectors.

---

## 4. Documented Volatile Fields and Comparator Tolerances

The `scripts/compare.py` script validates captures pixel-by-pixel, enforcing strict tolerances to differentiate legitimate variations from visual regressions:

### Tolerance Thresholds
1. **Per-Channel Color Tolerance (`COLOR_TOLERANCE = 12`):**
   - Color variations with a maximum delta `<= 12` across any channel (0–255 scale) are ignored.
   - *Reason:* Absorbs imperceptible sub-pixel antialiasing and font smoothing differences across browser rendering engine versions without masking intentional token changes or border/focus colors (whose deltas exceed 50).
2. **Antialiasing Noise Tolerance (`MAX_UNEXPECTED_PIXELS = 15`):**
   - Residual tolerance of up to 15 pixels outside volatile regions (approx. 0.001% of the 1,296,000 px area).
   - *Reason:* Prevents false positives from 1 px micro-fringes on font curves and rounded corners, while ensuring immediate failure on any structural layout shift (a 2 px margin shift affects hundreds of pixels).

### Allowed Volatile Regions per Screen

Any alteration outside the bounding boxes below results in `exit code 1`, reporting the count of diverging pixels and the anomaly's bounding box:

| Screen | Component / Element | Bounding Box `[x, y, w, h]` | Reason for Volatility |
|---|---|---|---|
| `01-login.png` | *(None)* | *(Zero tolerance)* | 100% static and deterministic screen. |
| `02-orgs-lista.png` | Topbar: Engine Chip (`.poph-topbar__estado`) | `[1050, 10, 214, 38]` | Dynamic process RSS (`Engine: 200 OK · [N] MB`). |
| `02-orgs-lista.png` | Orgs Telemetry (`.poph-orgs__telemetria`) | `[230, 72, 270, 24]` | Cluster `Uptime` counter and `RAM` usage. |
| `03-orgs-inspetor.png` | Topbar: Engine Chip (`.poph-topbar__estado`) | `[1050, 10, 214, 38]` | Dynamic process RSS. |
| `03-orgs-inspetor.png` | Orgs Telemetry (`.poph-orgs__telemetria`) | `[230, 72, 270, 24]` | Cluster `Uptime` counter and `RAM` usage. |
| `04-agente-instrucoes.png` | Topbar: Engine Chip (`.poph-topbar__estado`) | `[1050, 10, 214, 38]` | Dynamic process RSS. |
| `05-agente-base-conhecimento.png` | Topbar: Engine Chip (`.poph-topbar__estado`) | `[1050, 10, 214, 38]` | Dynamic process RSS. |
| `06-chat-teste.png` | Topbar: Engine Chip (`.poph-topbar__estado`) | `[1050, 10, 214, 38]` | Dynamic process RSS. |
| `07-operacao.png` | Topbar: Engine Chip (`.poph-topbar__estado`) | `[1050, 10, 214, 38]` | Dynamic process RSS. |
| `07-operacao.png` | Timestamp (`.poph-operacao__atualizacao`) | `[1170, 100, 134, 36]` | Dynamic query timestamp (`Atualizado às HH:MM`). |
| `07-operacao.png` | Operation: Process Uptime | `[730, 298, 150, 20]` | Elapsed seconds since daemon startup. |
| `07-operacao.png` | Operation: Process Memory | `[730, 663, 150, 20]` | Dynamic server heap memory usage. |

---

## 5. Starting the Console and Repeating Captures

### Prerequisites
- Compiled `poph` binary (`cargo build -p poph`)
- Node.js (>= 22.18) and monorepo dependencies (`playwright` installed in workspace)
- Google Chrome or Chromium

### Automated Reproduction (Recommended)
From the root of the `apps/pop-harness` repository:

```bash
bash design_assets/20261123-refactor-baseline/scripts/capturar.sh
```

The script:
1. Creates an isolated temporary directory and runs schema migrations (`poph migrate`).
2. Seeds deterministic demo data (`seed.py`).
3. Starts `poph serve` on a dedicated port (default: `17485`).
4. Captures the 7 screens via Playwright (`capture.mjs`).
5. Executes a second capture run and runs the stability comparison (`compare.py`), proving zero regressions outside documented volatile fields.
