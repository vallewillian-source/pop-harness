// Capturas do Element para o aceite do módulo Matrix (MX-31), sem Playwright.
//
// Fala com o Chrome por CDP (WebSocket nativo do Node 22). O perfil fica em
// <diretorio>/chrome-profile, então o login é feito uma vez e as execuções seguintes
// reaproveitam a sessão. Se o login cair na tela de verificação de dispositivo, ela é
// resolvida no fluxo "Não consegue confirmar?" -> Continuar -> senha (conta descartável
// do Synapse de desenvolvimento; o Element não é alterado).
//
// Uso:
//   node aceite-matrix-element.mjs a1    <saida> <sala> <espaco> <diretorio>
//   node aceite-matrix-element.mjs final <saida> <sala> <espaco> <diretorio> [sala-do-nome-tardio]
//
// `a1`   captura a sala recém-criada (nome do cliente) e o espaço do departamento ANTES de
//        qualquer transferência — é a prova de que a sala nasce nomeada e já dentro do espaço.
// `final` captura o estado depois de resposta do atendente, transferência, reabertura e
//        fechamento, a home e, quando informada, a sala cujo nome chegou depois (MX-32/MX-33).
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [MODO, SAIDA, SALA, ESPACO, DIR, SALA2] = process.argv.slice(2);
const CHROME = process.env.CHROME_BIN || 'google-chrome';
const BASE = process.env.ACEITE_ELEMENT || 'http://127.0.0.1:18403';
const SENHA = process.env.ACEITE_SENHA || readFileSync(join(DIR, 'atendente.senha'), 'utf8').trim();
const USUARIO = process.env.ACEITE_USUARIO || readFileSync(join(DIR, 'atendente.usuario'), 'utf8').trim();
// Perfil por passada: duas passadas em sequência não disputam o mesmo `user-data-dir` (o Chrome
// recusa subir com o perfil de uma instância que ainda está encerrando). Cada passada faz o login.
const PERFIL = join(DIR, 'chrome-profile-' + MODO);
mkdirSync(SAIDA, { recursive: true });
mkdirSync(PERFIL, { recursive: true });

// ───────────────────────────── driver CDP mínimo ─────────────────────────────
// O perfil é reaproveitado entre passadas (a1 e final). Um Chrome morto à força pode deixar o
// `DevToolsActivePort` e os `Singleton*` para trás; sem limpar, a passada seguinte lê a porta da
// anterior e cai na conexão. Limpa antes de subir.
const arquivoPorta = join(PERFIL, 'DevToolsActivePort');
for (const resto of [arquivoPorta, 'SingletonLock', 'SingletonCookie', 'SingletonSocket']) {
  try { rmSync(join(PERFIL, resto), { force: true }); } catch {}
}
const proceso = spawn(CHROME, [
  '--headless=new', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage',
  '--remote-debugging-port=0', `--user-data-dir=${PERFIL}`, '--window-size=1440,900',
  '--hide-scrollbars', '--force-device-scale-factor=1', '--no-first-run',
  '--no-default-browser-check', '--remote-allow-origins=*', 'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] });
let erroChrome = '';
proceso.stderr.on('data', (d) => { erroChrome += d.toString(); });

for (let i = 0; i < 400 && !existsSync(arquivoPorta); i++) await new Promise((r) => setTimeout(r, 50));
if (!existsSync(arquivoPorta)) {
  console.error('sem DevToolsActivePort', erroChrome.slice(-800));
  process.exit(1);
}
const [porta, caminhoWs] = readFileSync(arquivoPorta, 'utf8').split('\n');
const ws = new WebSocket(`ws://127.0.0.1:${porta}${caminhoWs}`);
await new Promise((ok, falha) => {
  ws.onopen = ok;
  ws.onerror = () => falha(new Error(`CDP não conectou em 127.0.0.1:${porta}; chrome: ${erroChrome.slice(-400)}`));
});
// já conectado: erro posterior vira timeout nos comandos, não derruba o processo com evento solto
ws.onerror = () => {};

let id = 0;
const pendentes = new Map();
const ouvintes = [];
ws.onmessage = (ev) => {
  const msg = JSON.parse(ev.data);
  if (msg.id && pendentes.has(msg.id)) {
    const { ok, falha } = pendentes.get(msg.id);
    pendentes.delete(msg.id);
    msg.error ? falha(new Error(JSON.stringify(msg.error))) : ok(msg.result);
  } else {
    ouvintes.forEach((f) => f(msg));
  }
};
const send = (method, params = {}, sessionId) => new Promise((ok, falha) => {
  const i = ++id;
  pendentes.set(i, { ok, falha });
  ws.send(JSON.stringify({ id: i, method, params, ...(sessionId ? { sessionId } : {}) }));
});
const evento = (method, sessionId, timeout = 20000) => new Promise((ok, falha) => {
  const t = setTimeout(() => falha(new Error('timeout ' + method)), timeout);
  const f = (m) => {
    if (m.method === method && (!sessionId || m.sessionId === sessionId)) {
      clearTimeout(t);
      ouvintes.splice(ouvintes.indexOf(f), 1);
      ok(m.params);
    }
  };
  ouvintes.push(f);
});

const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
await send('Page.enable', {}, sessionId);
await send('Runtime.enable', {}, sessionId);

const dormir = (ms) => new Promise((r) => setTimeout(r, ms));
const avaliar = async (expr) => {
  const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true }, sessionId);
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
  return r.result.value;
};
const irPara = async (url) => {
  await send('Page.navigate', { url }, sessionId);
  try { await evento('Page.loadEventFired', sessionId, 15000); } catch {}
  await dormir(2500);
};
const capturar = async (arquivo) => {
  const r = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false }, sessionId);
  writeFileSync(join(SAIDA, arquivo), Buffer.from(r.data, 'base64'));
  console.log('captura', join(SAIDA, arquivo));
};
const esperar = async (cond, ms = 25000, passo = 500) => {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    if (await avaliar(cond)) return true;
    await dormir(passo);
  }
  return false;
};
const clicarTexto = (txt) => avaliar(`(()=>{const el=[...document.querySelectorAll('button,a,span,[role=button]')].find(e=>e.textContent.trim()===${JSON.stringify(txt)});if(el){el.click();return 'ok'}return 'nao'})()`);
const preencher = (sel, valor) => avaliar(`(()=>{const el=document.querySelector(${JSON.stringify(sel)});if(!el)return 'sem';const d=Object.getOwnPropertyDescriptor(el.__proto__,'value');d.set.call(el,${JSON.stringify(valor)});el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));return 'ok'})()`);

async function logado() {
  const texto = await avaliar('document.body.innerText');
  return !texto.includes('Be in your element') && !texto.includes('Nome de usuário') && !texto.includes('Confirme sua identidade');
}

async function login() {
  await irPara(BASE + '/');
  await dormir(3500);
  if (await logado()) return true;

  // tela inicial
  await clicarTexto('Entrar');
  await dormir(2500);
  if (await avaliar(`!!document.querySelector('#mx_LoginForm_username')`)) {
    await preencher('#mx_LoginForm_username', USUARIO);
    await preencher('#mx_LoginForm_password', SENHA);
    await dormir(400);
    await avaliar(`(()=>{const b=[...document.querySelectorAll('button')].find(e=>e.type==='submit');if(b)b.click();return !!b})()`);
    await dormir(8000);
  }

  // verificação de dispositivo (conta nova)
  for (let tentativa = 0; tentativa < 3; tentativa++) {
    const texto = await avaliar('document.body.innerText');
    if (texto.includes('M_LIMIT_EXCEEDED')) {
      console.log('rate limit no login; esperando 60 s');
      await dormir(60000);
      await irPara(BASE + '/');
      await dormir(3000);
      await clicarTexto('Entrar');
      await dormir(2000);
      await preencher('#mx_LoginForm_username', USUARIO);
      await preencher('#mx_LoginForm_password', SENHA);
      await avaliar(`(()=>{const b=[...document.querySelectorAll('button')].find(e=>e.type==='submit');if(b)b.click()})()`);
      await dormir(8000);
      continue;
    }
    if (texto.includes('Confirme sua identidade') || texto.includes('Verifique este dispositivo')) {
      if (texto.includes('digite sua senha')) {
        await avaliar(`(()=>{const i=[...document.querySelectorAll('input')].find(x=>x.type==='password');if(!i)return false;const d=Object.getOwnPropertyDescriptor(i.__proto__,'value');d.set.call(i,${JSON.stringify(SENHA)});i.dispatchEvent(new Event('input',{bubbles:true}));i.dispatchEvent(new Event('change',{bubbles:true}));return true})()`);
        await dormir(500);
        await avaliar(`(()=>{const b=document.querySelector('input[type=submit],button[type=submit]');if(b){b.click();return true}return false})()`);
        await dormir(9000);
        continue;
      }
      await clicarTexto('Não consegue confirmar?');
      await dormir(1500);
      await clicarTexto('Continuar');
      await dormir(5000);
      continue;
    }
    break;
  }
  await irPara(BASE + '/');
  await dormir(4000);
  return logado();
}

// Fecha avisos do Element que cobrem a tela: "Confirmar esta sessão" (Mais tarde) e o tour
// "Introducing Sections" (Ok). Nenhum deles muda o app; só atrapalham a captura.
const limparAvisos = async () => {
  await clicarTexto('Mais tarde');
  await dormir(400);
  await clicarTexto('Ok');
  await dormir(400);
  await clicarTexto('Dispensar');
  await dormir(400);
};

const capturarSala = async (sala, arquivo) => {
  await irPara(BASE + '/#/room/' + encodeURIComponent(sala));
  await esperar(`document.querySelectorAll('.mx_EventTile').length > 0`, 35000);
  await dormir(2500);
  await limparAvisos();
  await dormir(500);
  await capturar(arquivo);
};

const capturarEspaco = async (espaco, arquivo) => {
  await irPara(BASE + '/#/room/' + encodeURIComponent(espaco));
  await dormir(6000);
  await limparAvisos();
  await dormir(1500);
  await capturar(arquivo);
};

const saida = [];
try {
  const ok = await login();
  if (!ok) throw new Error('não entrou no Element');
  await dormir(2000);
  // modais do primeiro login (recuperação de chave e notificações) não afetam a prova
  await clicarTexto('Not now');
  await dormir(800);
  await limparAvisos();

  if (MODO === 'a1') {
    await capturarSala(SALA, 'a1-sala.png');
    saida.push('a1-sala.png');
    await capturarEspaco(ESPACO, 'a1-espaco.png');
    saida.push('a1-espaco.png');
  } else if (MODO === 'final') {
    await capturarSala(SALA, 'sala.png');
    saida.push('sala.png');
    await capturarEspaco(ESPACO, 'espaco.png');
    saida.push('espaco.png');
    if (SALA2) {
      await capturarSala(SALA2, 'sala-nome-depois.png');
      saida.push('sala-nome-depois.png');
    }
    await irPara(BASE + '/#/home');
    await dormir(4000);
    await capturar('home.png');
    saida.push('home.png');
  } else {
    throw new Error('modo desconhecido: ' + MODO);
  }

  writeFileSync(join(SAIDA, 'capturas.txt'), saida.join('\n') + '\n');
  console.log('capturas:', saida.join(', '));
} catch (e) {
  console.error('ERRO', e.message);
  try { await capturar('falha.png'); } catch {}
  process.exitCode = 1;
} finally {
  try { ws.close(); } catch {}
  await new Promise((ok) => {
    const t = setTimeout(ok, 4000);
    proceso.once('exit', () => { clearTimeout(t); ok(); });
    proceso.kill('SIGKILL');
  });
}
