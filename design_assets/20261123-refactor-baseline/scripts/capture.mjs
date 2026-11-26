#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const ROOT = path.resolve(__dirname, "../../..");
const OUT_DIR = path.resolve(__dirname, "..");

let playwright;
const candidatePaths = [
    process.env.PLAYWRIGHT_PATH,
    path.resolve(ROOT, "../frontend/node_modules/playwright"),
    path.resolve(ROOT, "../../apps/frontend/node_modules/playwright"),
    path.resolve(ROOT, "node_modules/playwright"),
    "playwright"
].filter(Boolean);

for (const p of candidatePaths) {
    try {
        playwright = require(p);
        break;
    } catch (e) {
    }
}

if (!playwright || !playwright.chromium) {
    console.error("Não foi possível carregar o Playwright a partir dos caminhos candidatos.");
    process.exit(1);
}

const { chromium } = playwright;
const REQUIRED_PORT = "17485";
const PORT = process.env.POPH_PORT || REQUIRED_PORT;
if (PORT !== REQUIRED_PORT) {
    console.error(`Erro: A porta deve ser obrigatoriamente ${REQUIRED_PORT} (recebido: ${PORT}) pois a tela de login exibe a porta na linha de base visual.`);
    process.exit(1);
}

const BASE_URL = `http://127.0.0.1:${PORT}`;
const DEMO_TOKEN = "pophg_ae9fb56344790490b97e1e47618c4a8fcf97150989667f52a19161ef5bc172ef";

async function waitForServer(url, timeoutMs = 15000) {
    const start = Date.now();
    while (Date.now() - start < timeoutMs) {
        try {
            const res = await fetch(`${url}/healthz`);
            if (res.ok) return true;
        } catch (_) {
        }
        await new Promise((r) => setTimeout(r, 200));
    }
    throw new Error(`Servidor não respondeu em ${url}/healthz após ${timeoutMs}ms`);
}

async function waitScreenReady(page, contentSelector) {
    if (contentSelector) {
        await page.waitForSelector(contentSelector, { state: "visible" });
    }
    await page.evaluate(() => document.fonts.ready);
    await page.waitForFunction(() => {
        const busy = document.querySelectorAll('[aria-busy="true"]');
        const spinners = document.querySelectorAll('.sofa-spinner');
        const skeletons = document.querySelectorAll('.sofa-skeleton, .poph-orgs__skeleton, .poph-knowledge__skeleton-line');
        return busy.length === 0 && spinners.length === 0 && skeletons.length === 0;
    });
    await page.evaluate(() => {
        if (document.activeElement && typeof document.activeElement.blur === "function") {
            document.activeElement.blur();
        }
    });
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
}

async function captureScreens(targetDir) {
    fs.mkdirSync(targetDir, { recursive: true });
    await waitForServer(BASE_URL);

    const chromePath = process.env.CHROME_BIN || (fs.existsSync("/usr/bin/google-chrome") ? "/usr/bin/google-chrome" : undefined);
    const browser = await chromium.launch({
        executablePath: chromePath,
        headless: true,
        args: ["--no-sandbox", "--disable-gpu", "--force-device-scale-factor=1"]
    });

    const context = await browser.newContext({
        viewport: { width: 1440, height: 900 },
        deviceScaleFactor: 1
    });

    await context.addInitScript(() => {
        const inject = () => {
            if (document.head) {
                const style = document.createElement("style");
                style.textContent = `
                    *, *::before, *::after {
                        animation-duration: 0s !important;
                        animation-delay: 0s !important;
                        transition-duration: 0s !important;
                        transition-delay: 0s !important;
                        caret-color: transparent !important;
                    }
                `;
                document.head.appendChild(style);
            }
        };
        if (document.readyState === "loading") {
            document.addEventListener("DOMContentLoaded", inject);
        } else {
            inject();
        }
    });

    const page = await context.newPage();

    await page.goto(`${BASE_URL}/login`);
    await page.waitForSelector("input#poph-token", { state: "visible" });
    await page.waitForSelector(".poph-fact__value--mono", { state: "visible" });
    await waitScreenReady(page, "button:has-text(\"Entrar\")");
    await page.screenshot({ path: path.join(targetDir, "01-login.png"), animations: "disabled" });
    fs.writeFileSync(path.join(targetDir, "01-login.html"), await page.content());
    console.log("✓ Capturada tela 1: Login (01-login.png)");

    await page.fill("input#poph-token", DEMO_TOKEN);
    await page.click("button:has-text(\"Entrar\")");
    await page.waitForURL(/\/orgs(\/[^/]+)?$/);
    await page.evaluate(() => window.localStorage.removeItem("poph-console.lastOrgId"));
    await page.goto(`${BASE_URL}/orgs`);
    await page.waitForURL("**/orgs");
    await page.waitForSelector("tr:has-text(\"Acme Demonstração\")", { state: "visible" });
    await page.waitForSelector(".poph-orgs__telemetry", { state: "visible" });
    await page.waitForSelector("button[aria-label=\"Atualizar\"]:not([aria-busy=\"true\"])", { state: "visible" });
    await waitScreenReady(page, ".poph-orgs__table");
    await page.screenshot({ path: path.join(targetDir, "02-orgs-lista.png"), animations: "disabled" });
    fs.writeFileSync(path.join(targetDir, "02-orgs-lista.html"), await page.content());
    console.log("✓ Capturada tela 2: Orgs - lista (02-orgs-lista.png)");

    const token = await page.evaluate(() => {
        const item = sessionStorage.getItem("poph-console.sessao");
        return JSON.parse(item).token;
    });

    const ids = await page.evaluate(async (tok) => {
        const r = await fetch("/admin/v1/orgs?cluster=true", { headers: { Authorization: "Bearer " + tok } });
        const data = await r.json();
        const org = data.items.find(o => o.slug === "acme") || data.items[0];
        const r2 = await fetch(`/admin/v1/orgs/${org.id}/agents`, { headers: { Authorization: "Bearer " + tok } });
        const data2 = await r2.json();
        return { orgId: org.id, agentId: data2.items[0].id };
    }, token);

    const orgRow = page.locator("tr", { hasText: "Acme Demonstração" }).first();
    await orgRow.click();
    await page.waitForSelector(".poph-inspector", { state: "visible" });
    await page.waitForSelector(".poph-inspector h2:has-text(\"Acme Demonstração\")", { state: "visible" });
    await waitScreenReady(page, ".poph-inspector__section");
    await page.screenshot({ path: path.join(targetDir, "03-orgs-inspetor.png"), animations: "disabled" });
    fs.writeFileSync(path.join(targetDir, "03-orgs-inspetor.html"), await page.content());
    console.log("✓ Capturada tela 3: Orgs - inspetor (03-orgs-inspetor.png)");

    await page.goto(`${BASE_URL}/orgs/${ids.orgId}/agents/${ids.agentId}/instructions`);
    await page.waitForSelector(".poph-rail__agent:has-text(\"Agente: Bia Atendente\")", { state: "visible" });
    await page.waitForSelector(".poph-topbar__state--agent:has-text(\"Modelo: gemini-3.8-flash-tiered\")", { state: "visible" });
    await page.waitForSelector("h1.poph-page__title:has-text(\"Bia Atendente\")", { state: "visible" });
    await page.waitForSelector(".poph-agent__identity", { state: "visible" });
    await page.waitForSelector("textarea", { state: "visible" });
    await waitScreenReady(page, ".poph-instructions, .poph-agent__identity");
    await page.screenshot({ path: path.join(targetDir, "04-agente-instrucoes.png"), animations: "disabled" });
    fs.writeFileSync(path.join(targetDir, "04-agente-instrucoes.html"), await page.content());
    console.log("✓ Capturada tela 4: Agente - Instruções (04-agente-instrucoes.png)");

    await page.goto(`${BASE_URL}/orgs/${ids.orgId}/agents/${ids.agentId}/knowledge-base`);
    await page.waitForSelector(".poph-rail__agent:has-text(\"Agente: Bia Atendente\")", { state: "visible" });
    await page.waitForSelector(".poph-topbar__state--agent:has-text(\"Modelo: gemini-3.8-flash-tiered\")", { state: "visible" });
    await page.waitForSelector("h1.poph-page__title:has-text(\"Bia Atendente\")", { state: "visible" });
    await page.waitForSelector(".poph-knowledge", { state: "visible" });
    await page.waitForSelector("h2.poph-section__title:has-text(\"Índice de busca do agente\")", { state: "visible" });
    await page.waitForSelector(".poph-knowledge__groups", { state: "visible" });
    await waitScreenReady(page, ".poph-knowledge__groups");
    await page.screenshot({ path: path.join(targetDir, "05-agente-base-conhecimento.png"), animations: "disabled" });
    fs.writeFileSync(path.join(targetDir, "05-agente-base-conhecimento.html"), await page.content());
    console.log("✓ Capturada tela 5: Agente - Base de conhecimento (05-agente-base-conhecimento.png)");

    await page.goto(`${BASE_URL}/orgs/${ids.orgId}/agents/${ids.agentId}/test-chat`);
    await page.waitForSelector(".poph-rail__agent:has-text(\"Agente: Bia Atendente\")", { state: "visible" });
    await page.waitForSelector(".poph-topbar__state--agent:has-text(\"Modelo: gemini-3.8-flash-tiered\")", { state: "visible" });
    await page.waitForSelector("h1.poph-page__title:has-text(\"Bia Atendente\")", { state: "visible" });
    await page.waitForSelector(".poph-chat-test", { state: "visible" });
    await page.waitForSelector(".poph-chat-test__item", { state: "visible" });
    await page.locator(".poph-chat-test__item").first().click();
    await page.waitForSelector(".poph-chat-test__bubble", { state: "visible" });
    await page.waitForSelector("textarea#chat-teste-texto", { state: "visible" });
    await waitScreenReady(page, ".poph-chat-test__bubble");
    await page.screenshot({ path: path.join(targetDir, "06-chat-teste.png"), animations: "disabled" });
    fs.writeFileSync(path.join(targetDir, "06-chat-teste.html"), await page.content());
    console.log("✓ Capturada tela 6: Chat de teste (06-chat-teste.png)");

    await page.goto(`${BASE_URL}/orgs/${ids.orgId}/operacao`);
    await page.waitForSelector(".poph-operation", { state: "visible" });
    await page.waitForSelector("h1:has-text(\"Saúde da instância\")", { state: "visible" });
    await page.waitForSelector(".poph-operation__update", { state: "visible" });
    await waitScreenReady(page, ".poph-operation");
    await page.screenshot({ path: path.join(targetDir, "07-operacao.png"), animations: "disabled" });
    fs.writeFileSync(path.join(targetDir, "07-operacao.html"), await page.content());
    console.log("✓ Capturada tela 7: Operação (07-operacao.png)");

    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto(`${BASE_URL}/orgs/${ids.orgId}/agents/${ids.agentId}/instructions`);
    await page.waitForSelector(".poph-rail__agent:has-text(\"Agente: Bia Atendente\")", { state: "visible" });
    await page.waitForSelector(".poph-topbar__state--agent:has-text(\"Modelo: gemini-3.8-flash-tiered\")", { state: "visible" });
    await page.waitForSelector("h1.poph-page__title:has-text(\"Bia Atendente\")", { state: "visible" });
    await page.waitForSelector(".poph-agent__identity", { state: "visible" });
    await page.waitForSelector("textarea", { state: "visible" });
    await waitScreenReady(page, ".poph-agent__identity");
    await page.screenshot({ path: path.join(targetDir, "08-agente-instrucoes-1280.png"), animations: "disabled" });
    fs.writeFileSync(path.join(targetDir, "08-agente-instrucoes-1280.html"), await page.content());
    console.log("✓ Capturada tela 8: Agente - Instruções 1280px (08-agente-instrucoes-1280.png)");

    await browser.close();
}

async function main() {
    const target = process.argv[2] ? path.resolve(process.argv[2]) : OUT_DIR;
    await captureScreens(target);
}

main().catch((err) => {
    console.error("Erro na captura:", err);
    process.exit(1);
});
