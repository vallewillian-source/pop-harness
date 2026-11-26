import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const [MODE, OUTPUT, ROOM, SPACE, DIR, ROOM2] = process.argv.slice(2);
const CHROME = process.env.CHROME_BIN || 'google-chrome';
const BASE = process.env.ACEITE_ELEMENT || 'http://127.0.0.1:18403';
const PASSWORD = process.env.ACEITE_SENHA || readFileSync(join(DIR, 'atendente.senha'), 'utf8').trim();
const USER = process.env.ACEITE_USUARIO || readFileSync(join(DIR, 'atendente.usuario'), 'utf8').trim();
const PROFILE = join(DIR, 'chrome-profile-' + MODE);
mkdirSync(OUTPUT, { recursive: true });
mkdirSync(PROFILE, { recursive: true });

const portFile = join(PROFILE, 'DevToolsActivePort');
for (const rest of [portFile, 'SingletonLock', 'SingletonCookie', 'SingletonSocket']) {
  try { rmSync(join(PROFILE, rest), { force: true }); } catch {}
}
const chromeProcess = spawn(CHROME, [
  '--headless=new', '--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage',
  '--remote-debugging-port=0', `--user-data-dir=${PROFILE}`, '--window-size=1440,900',
  '--hide-scrollbars', '--force-device-scale-factor=1', '--no-first-run',
  '--no-default-browser-check', '--remote-allow-origins=*', 'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] });
let chromeError = '';
chromeProcess.stderr.on('data', (d) => { chromeError += d.toString(); });

for (let i = 0; i < 400 && !existsSync(portFile); i++) await new Promise((r) => setTimeout(r, 50));
if (!existsSync(portFile)) {
  console.error('no DevToolsActivePort', chromeError.slice(-800));
  process.exit(1);
}
const [port, wsPath] = readFileSync(portFile, 'utf8').split('\n');
const ws = new WebSocket(`ws://127.0.0.1:${port}${wsPath}`);
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = () => reject(new Error(`CDP did not connect to 127.0.0.1:${port}; chrome: ${chromeError.slice(-400)}`));
});
ws.onerror = () => {};

let id = 0;
const pending = new Map();
const listeners = [];
ws.onmessage = (ev) => {
  const msg = JSON.parse(ev.data);
  if (msg.id && pending.has(msg.id)) {
    const { resolve, reject } = pending.get(msg.id);
    pending.delete(msg.id);
    msg.error ? reject(new Error(JSON.stringify(msg.error))) : resolve(msg.result);
  } else {
    listeners.forEach((f) => f(msg));
  }
};
const send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
  const i = ++id;
  pending.set(i, { resolve, reject });
  ws.send(JSON.stringify({ id: i, method, params, ...(sessionId ? { sessionId } : {}) }));
});
const event = (method, sessionId, timeout = 20000) => new Promise((resolve, reject) => {
  const t = setTimeout(() => reject(new Error('timeout ' + method)), timeout);
  const f = (m) => {
    if (m.method === method && (!sessionId || m.sessionId === sessionId)) {
      clearTimeout(t);
      listeners.splice(listeners.indexOf(f), 1);
      resolve(m.params);
    }
  };
  listeners.push(f);
});

const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
await send('Page.enable', {}, sessionId);
await send('Runtime.enable', {}, sessionId);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const evaluate = async (expr) => {
  const r = await send('Runtime.evaluate', { expression: expr, awaitPromise: true, returnByValue: true }, sessionId);
  if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
  return r.result.value;
};
const goTo = async (url) => {
  await send('Page.navigate', { url }, sessionId);
  try { await event('Page.loadEventFired', sessionId, 15000); } catch {}
  await sleep(2500);
};
const capture = async (file) => {
  const r = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false }, sessionId);
  writeFileSync(join(OUTPUT, file), Buffer.from(r.data, 'base64'));
  console.log('capture', join(OUTPUT, file));
};
const waitFor = async (condition, ms = 25000, step = 500) => {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    if (await evaluate(condition)) return true;
    await sleep(step);
  }
  return false;
};
const clickText = (label) => evaluate(`(()=>{const el=[...document.querySelectorAll('button,a,span,[role=button]')].find(e=>e.textContent.trim()===${JSON.stringify(label)});if(el){el.click();return 'ok'}return 'no'})()`);
const fill = (selector, value) => evaluate(`(()=>{const el=document.querySelector(${JSON.stringify(selector)});if(!el)return 'missing';const d=Object.getOwnPropertyDescriptor(el.__proto__,'value');d.set.call(el,${JSON.stringify(value)});el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));return 'ok'})()`);

async function isLoggedIn() {
  const text = await evaluate('document.body.innerText');
  return !text.includes('Be in your element') && !text.includes('Nome de usuário') && !text.includes('Confirme sua identidade');
}

async function login() {
  await goTo(BASE + '/');
  await sleep(3500);
  if (await isLoggedIn()) return true;

  await clickText('Entrar');
  await sleep(2500);
  if (await evaluate(`!!document.querySelector('#mx_LoginForm_username')`)) {
    await fill('#mx_LoginForm_username', USER);
    await fill('#mx_LoginForm_password', PASSWORD);
    await sleep(400);
    await evaluate(`(()=>{const b=[...document.querySelectorAll('button')].find(e=>e.type==='submit');if(b)b.click();return !!b})()`);
    await sleep(8000);
  }

  for (let attempt = 0; attempt < 3; attempt++) {
    const text = await evaluate('document.body.innerText');
    if (text.includes('M_LIMIT_EXCEEDED')) {
      console.log('rate limit on login; waiting 60 s');
      await sleep(60000);
      await goTo(BASE + '/');
      await sleep(3000);
      await clickText('Entrar');
      await sleep(2000);
      await fill('#mx_LoginForm_username', USER);
      await fill('#mx_LoginForm_password', PASSWORD);
      await evaluate(`(()=>{const b=[...document.querySelectorAll('button')].find(e=>e.type==='submit');if(b)b.click()})()`);
      await sleep(8000);
      continue;
    }
    if (text.includes('Confirme sua identidade') || text.includes('Verifique este dispositivo')) {
      if (text.includes('digite sua senha')) {
        await evaluate(`(()=>{const i=[...document.querySelectorAll('input')].find(x=>x.type==='password');if(!i)return false;const d=Object.getOwnPropertyDescriptor(i.__proto__,'value');d.set.call(i,${JSON.stringify(PASSWORD)});i.dispatchEvent(new Event('input',{bubbles:true}));i.dispatchEvent(new Event('change',{bubbles:true}));return true})()`);
        await sleep(500);
        await evaluate(`(()=>{const b=document.querySelector('input[type=submit],button[type=submit]');if(b){b.click();return true}return false})()`);
        await sleep(9000);
        continue;
      }
      await clickText('Não consegue confirmar?');
      await sleep(1500);
      await clickText('Continuar');
      await sleep(5000);
      continue;
    }
    break;
  }
  await goTo(BASE + '/');
  await sleep(4000);
  return isLoggedIn();
}

const clearNotices = async () => {
  await clickText('Mais tarde');
  await sleep(400);
  await clickText('Ok');
  await sleep(400);
  await clickText('Dispensar');
  await sleep(400);
};

const captureRoom = async (room, file) => {
  await goTo(BASE + '/#/room/' + encodeURIComponent(room));
  await waitFor(`document.querySelectorAll('.mx_EventTile').length > 0`, 35000);
  await sleep(2500);
  await clearNotices();
  await sleep(500);
  await capture(file);
};

const captureSpace = async (space, file) => {
  await goTo(BASE + '/#/room/' + encodeURIComponent(space));
  await sleep(6000);
  await clearNotices();
  await sleep(1500);
  await capture(file);
};

const captured = [];
try {
  const logged = await login();
  if (!logged) throw new Error('could not log in to Element');
  await sleep(2000);
  await clickText('Not now');
  await sleep(800);
  await clearNotices();

  if (MODE === 'a1') {
    await captureRoom(ROOM, 'a1-sala.png');
    captured.push('a1-sala.png');
    await captureSpace(SPACE, 'a1-espaco.png');
    captured.push('a1-espaco.png');
  } else if (MODE === 'final') {
    await captureRoom(ROOM, 'sala.png');
    captured.push('sala.png');
    await captureSpace(SPACE, 'espaco.png');
    captured.push('espaco.png');
    if (ROOM2) {
      await captureRoom(ROOM2, 'sala-nome-depois.png');
      captured.push('sala-nome-depois.png');
    }
    await goTo(BASE + '/#/home');
    await sleep(4000);
    await capture('home.png');
    captured.push('home.png');
  } else {
    throw new Error('unknown mode: ' + MODE);
  }

  writeFileSync(join(OUTPUT, 'capturas.txt'), captured.join('\n') + '\n');
  console.log('captures:', captured.join(', '));
} catch (e) {
  console.error('ERROR', e.message);
  try { await capture('falha.png'); } catch {}
  process.exitCode = 1;
} finally {
  try { ws.close(); } catch {}
  await new Promise((resolve) => {
    const t = setTimeout(resolve, 4000);
    chromeProcess.once('exit', () => { clearTimeout(t); resolve(); });
    chromeProcess.kill('SIGKILL');
  });
}
