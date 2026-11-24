# Console Visual Baseline (RF-08)

This directory holds the definitive visual baseline for the primary screens of the POP Harness Console, captured following the completion and acceptance of the final layout enhancements (CF-09 agent cockpit rail, CF-10 contextual topbar, and CF-11 three-step breadcrumb without token badge).

The purpose of this baseline is to serve as an immutable comparative visual reference ensuring exact parity of layout, typography, spacing, and components during and after frontend refactors and renamings (English codebase and comment removal).

---

## 1. Captured Screens

All captures were generated in PNG format along with their respective DOM HTML snapshots:

| File | Screen / Route | Captured State |
|---|---|---|
| `01-login.png` | Login (`/login`) | Clean initial state: admin token field, in-memory usage guidance, and submit button. |
| `02-orgs-lista.png` | Organizations (`/orgs`) | Organization table with demo seed data, filter tabs ("All", "Active", "Inactive"), global search, cluster status, and actions. |
| `03-orgs-inspetor.png` | Organization Inspector (`/orgs`) | Side inspector drawer opened over the selected organization, displaying metadata, CLI snippet, active agent card, node status, and actions. |
| `04-agente-instrucoes.png` | Agent / Instructions (`/orgs/:id/agents/:id/instructions`) | 1440 px canonical view: agent cockpit rail with 7 items, 3-step breadcrumb (`Acme Demonstração › Bia Atendente › Instruções & Prompts`), contextual search placeholder, agent model chip (`● Modelo: gemini-3.8-flash-tiered 200 OK`), "Sair" button with label, and prompt form. |
| `05-agente-base-conhecimento.png` | Agent / Knowledge Base (`/orgs/:id/agents/:id/knowledge-base`) | 1440 px view: hybrid search view and table of knowledge groups linked to the agent with the new agent cockpit rail and header. |
| `06-chat-teste.png` | Test Chat (`/orgs/:id/agents/:id/test-chat`) | 1440 px view: active test session with message exchange (user and agent), status indicator, message composer, and agent cockpit rail. |
| `07-operacao.png` | Operation & Health (`/orgs/:id/operacao`) | Operational telemetry dashboard with engine health, runtime metrics, declared plugins, and cluster card. |
| `08-agente-instrucoes-1280.png` | Agent / Instructions — 1280 px (`/orgs/:id/agents/:id/instructions`) | Responsive 1280 px view: confirms that the 3-step breadcrumb remains fully visible without truncation, contextual search gracefully narrows, and the agent cockpit rail remains fully functional. |

---

## 2. Resolution and Viewport

Captures follow the reference desktop layout specification (`console/UX_SPEC.md`):

- **Canonical Desktop Viewport:** `1440 × 900` pixels (screens `01` through `07`)
- **Responsive Viewport:** `1280 × 900` pixels (screen `08`, enforcing three-step breadcrumb integrity)
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
| `02-orgs-lista.png` | Topbar: Engine Chip (`.poph-topbar__estado`) | `[1040, 10, 240, 38]` | Dynamic process RSS (`Engine: 200 OK · [N] MB`). |
| `02-orgs-lista.png` | Orgs Telemetry (`.poph-orgs__telemetria`) | `[1160, 180, 260, 25]` | Cluster `Uptime` counter and `RAM` usage (`1 nó conectado · Uptime 0m · RAM [N] MB`). |
| `03-orgs-inspetor.png` | Topbar: Engine Chip (`.poph-topbar__estado`) | `[1040, 10, 240, 38]` | Dynamic process RSS. |
| `03-orgs-inspetor.png` | Orgs Telemetry (`.poph-orgs__telemetria`) | `[780, 225, 260, 25]` | Cluster `Uptime` counter and `RAM` usage (wrapped below filter tabs due to open inspector drawer). |
| `04-agente-instrucoes.png` | *(None)* | *(Zero tolerance)* | **100% static:** In CF-10 / CF-11, the topbar indicator in agent routes was replaced by the `.poph-topbar__estado--agente` chip (`● Modelo: gemini-3.8-flash-tiered 200 OK`). Dynamic process memory RSS (`[N] MB`) was removed. Model is fixed from demo seed and health status is fixed at `200 OK`. |
| `05-agente-base-conhecimento.png` | *(None)* | *(Zero tolerance)* | **100% static:** Agent topbar indicator contains deterministic model and health status with no volatile process RSS. |
| `06-chat-teste.png` | *(None)* | *(Zero tolerance)* | **100% static:** Agent topbar indicator contains deterministic model and health status; seed conversation messages use fixed timestamps. |
| `07-operacao.png` | Topbar: Engine Chip (`.poph-topbar__estado`) | `[1040, 10, 240, 38]` | Dynamic process RSS (`Engine: 200 OK · [N] MB`). |
| `07-operacao.png` | Timestamp (`.poph-operacao__atualizacao`) | `[1170, 100, 134, 36]` | Dynamic query timestamp (`Atualizado às HH:MM`). |
| `07-operacao.png` | Operation: Process Uptime | `[730, 298, 150, 20]` | Elapsed seconds since daemon startup. |
| `07-operacao.png` | Operation: Process Memory | `[730, 663, 150, 20]` | Dynamic server heap memory usage. |
| `08-agente-instrucoes-1280.png` | *(None)* | *(Zero tolerance)* | **100% static:** Responsive 1280 px agent view verifying three-step breadcrumb (`Acme Demonstração › Bia Atendente › Instruções & Prompts`), contextual search placeholder, and agent model chip without volatile RSS. |

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
4. Warms up the instance to reach steady-state runtime heap and cached buffers.
5. Captures the 8 baseline screens via Playwright (`capture.mjs`) into `design_assets/20261123-refactor-baseline/`.
6. Executes a second capture run into a separate temporary directory.
7. Executes `compare.py` verifying pixel-by-pixel stability and zero regressions outside documented volatile fields.
