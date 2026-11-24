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
        // continue
    }
}

if (!playwright || !playwright.chromium) {
    console.error("Não foi possível carregar o Playwright a partir dos caminhos candidatos.");
    process.exit(1);
}

const { chromium } = playwright;
const PORT = process.env.POPH_PORT || "17480";
const BASE_URL = `http://127.0.0.1:${PORT}`;
const DEMO_TOKEN = "pophg_ae9fb56344790490b97e1e47618c4a8fcf97150989667f52a19161ef5bc172ef";

async function waitForServer(url, timeoutMs = 15000) {
    const start = Date.now();
    while (Date.now() - start < timeoutMs) {
        try {
            const res = await fetch(`${url}/healthz`);
            if (res.ok) return true;
        } catch (_) {
            // wait
        }
        await new Promise((r) => setTimeout(r, 200));
    }
    throw new Error(`Servidor não respondeu em ${url}/healthz após ${timeoutMs}ms`);
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

    const page = await context.newPage();

    // 1. Login
    await page.goto(`${BASE_URL}/login`);
    await page.waitForSelector("input");
    await page.waitForTimeout(600);
    await page.screenshot({ path: path.join(targetDir, "01-login.png") });
    fs.writeFileSync(path.join(targetDir, "01-login.html"), await page.content());
    console.log("✓ Capturada tela 1: Login (01-login.png)");

    // Autenticar com token global de demonstração
    await page.fill("input", DEMO_TOKEN);
    await page.click("button:has-text(\"Entrar\")");
    await page.waitForURL("**/orgs");
    await page.waitForTimeout(1000);

    // 2. Orgs lista
    await page.screenshot({ path: path.join(targetDir, "02-orgs-lista.png") });
    fs.writeFileSync(path.join(targetDir, "02-orgs-lista.html"), await page.content());
    console.log("✓ Capturada tela 2: Orgs - lista (02-orgs-lista.png)");

    // Obter IDs da org e agente para rotas profundas
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

    // 3. Orgs inspetor
    const orgRow = page.locator("tr", { hasText: "Acme Demonstração" }).first();
    await orgRow.click();
    await page.waitForSelector(".poph-inspetor");
    await page.waitForTimeout(600);
    await page.screenshot({ path: path.join(targetDir, "03-orgs-inspetor.png") });
    fs.writeFileSync(path.join(targetDir, "03-orgs-inspetor.html"), await page.content());
    console.log("✓ Capturada tela 3: Orgs - inspetor (03-orgs-inspetor.png)");

    // 4. Agente Instruções
    await page.goto(`${BASE_URL}/orgs/${ids.orgId}/agents/${ids.agentId}/instructions`);
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(targetDir, "04-agente-instrucoes.png") });
    fs.writeFileSync(path.join(targetDir, "04-agente-instrucoes.html"), await page.content());
    console.log("✓ Capturada tela 4: Agente - Instruções (04-agente-instrucoes.png)");

    // 5. Agente Base de conhecimento
    await page.goto(`${BASE_URL}/orgs/${ids.orgId}/agents/${ids.agentId}/knowledge-base`);
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(targetDir, "05-agente-base-conhecimento.png") });
    fs.writeFileSync(path.join(targetDir, "05-agente-base-conhecimento.html"), await page.content());
    console.log("✓ Capturada tela 5: Agente - Base de conhecimento (05-agente-base-conhecimento.png)");

    // 6. Chat de teste
    await page.goto(`${BASE_URL}/orgs/${ids.orgId}/agents/${ids.agentId}/test-chat`);
    await page.waitForTimeout(1000);
    const conv = page.locator(".poph-chat-teste__item").first();
    if (await conv.isVisible()) {
        await conv.click();
        await page.waitForTimeout(600);
    }
    await page.screenshot({ path: path.join(targetDir, "06-chat-teste.png") });
    fs.writeFileSync(path.join(targetDir, "06-chat-teste.html"), await page.content());
    console.log("✓ Capturada tela 6: Chat de teste (06-chat-teste.png)");

    // 7. Operação
    await page.goto(`${BASE_URL}/orgs/${ids.orgId}/operacao`);
    await page.waitForTimeout(1000);
    await page.screenshot({ path: path.join(targetDir, "07-operacao.png") });
    fs.writeFileSync(path.join(targetDir, "07-operacao.html"), await page.content());
    console.log("✓ Capturada tela 7: Operação (07-operacao.png)");

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
