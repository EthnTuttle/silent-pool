#!/usr/bin/env node
/* Owned, bounded TEST-ONLY controller/provider fixture. Candidate embedded
 * asset bytes, real Chromium/fetch/ReadableStream; no lab daemon is touched. */
"use strict";
const fs = require("node:fs"), path = require("node:path"), http = require("node:http");
const crypto = require("node:crypto"), { spawn } = require("node:child_process");
const assert = require("node:assert/strict");
const out = process.env.LIVE_UI_PROOF || "/tmp/opencode/live-wallet-ui-browser";
fs.mkdirSync(out,{recursive:true});
const root = path.resolve(__dirname,"../static");
const sha = data => crypto.createHash("sha256").update(data).digest("hex");
const assets = Object.fromEntries(["index.html","app.js","style.css","lab.html","lab.js","lab.css","access.js"].map(p => [p,fs.readFileSync(path.join(root,p))]));
const token = {operator:"a".repeat(64),alice:"b".repeat(64)};
let revoked = false, legacy = false, streams = 0;
const requests = [], commands = [], failures = [], exceptions = [], logs = [], responses = [];
const vtxo = "ab".repeat(36);
const entity = {id:"alice-worker",kind:"worker",miner_id:"alice-miner",status:"running",detail:"native SV2"};
const snapshot = user => ({schema:user ? "spc-user-snapshot/v1" : "spc-lab-snapshot/v1", run_id:"fixture-run",snapshot_cursor:7,retained_from:1,cursor:7,generated_at:Date.now(),filtered:!!user,
  entities:[entity],workers:[entity],watermarks:[],gaps:[],evidence:{gap_count:0,last_event_id:7,total_events:7}});
const inventory = () => ({schema:"spc.lab.inventory.v1",run_id:"fixture-run",lifecycle:"running",view:"live",topology:"host",recorded_live_actor_count:1,recorded_actor_count:1,declared_actor_count:1,
  latest_evidence:{age_ms:20,freshness:"fresh"},groups:[{id:"roles",title:"Lab role directory",purpose:"TEST fixture",rows:[{id:"role",process:'<img src=x onerror="window.XSS=1">',purpose:"fixture only",declared_role:"pool-a",logical_roles:["front"],recorded_state:"running",observation:{availability:"unknown"},dependencies:[],details:[],interfaces:[{kind:"operator_ui",url:"javascript:alert(1)",label:"hostile blocked URL"}]}]}],integrations:[]});
const server = http.createServer((req,res) => {
  // Never record Authorization values or request bodies containing credentials.
  requests.push({method:req.method,url:req.url});
  const staticName = req.url === "/" ? "index.html" : req.url === "/lab" ? "lab.html" : req.url.slice(1);
  if (req.method === "GET" && assets[staticName]) {
    res.writeHead(200,{"content-type":staticName.endsWith("html")?"text/html":staticName.endsWith("js")?"text/javascript":"text/css",
      "content-security-policy":"default-src 'self'; connect-src 'self'; form-action 'self'; base-uri 'none'", "cache-control":"no-store"}); res.end(assets[staticName]); return;
  }
  if (req.url === "/favicon.ico") return res.writeHead(204).end();
  const principal = legacy ? "operator" : Object.keys(token).find(k => req.headers.authorization === `Bearer ${token[k]}`);
  if (!principal || revoked) return res.writeHead(401).end("authentication required");
  const user = principal !== "operator";
  const json = value => {res.setHeader("content-type","application/json");res.end(JSON.stringify(value));};
  if (req.url.endsWith("/health")) return json({ok:true,run_id:"fixture-run"});
  if (req.url.endsWith("/me")) return json({schema:"spc-access-claim/v1",run_id:"fixture-run",principal,actor:user?"user/alice":"operator",may:user?["view_own_status","pause_resume_workers","set_worker_budget","wallet_refresh","wallet_offboard","send_user_payment"]:["operator"],identity_id:user?"alice":undefined,miner_id:user?"alice-miner":undefined,worker_ids:user?["alice-worker"]:undefined});
  if (req.url.includes("/snapshot")) return json(snapshot(user));
  if (req.url.endsWith("/inventory")) return user ? res.writeHead(403).end("forbidden") : json(inventory());
  if (req.url.endsWith("/catalog")) return user ? res.writeHead(403).end("forbidden") : json({fronts:[],identities:[],miners:[],defaults:{}});
  if (req.url.endsWith("/projection")) {
    const p={schema:"spc-user-projection/v1",user_id:"alice",observed_at:Date.now(),workers:[{worker_id:"alice-worker",requested_hps:10,achieved_hps:9,active_job_id:"job-7",observed_at:Date.now()}]};
    return json(user?p:[p]);
  }
  if (req.url.endsWith("/wallets")) return json({schema:"spc-lab-wallets/v1",state:"available",wallets:[{identity_id:"alice",pool_id:"pool-a",source:"TEST-ONLY agent provider fixture",observed_at_ms:Date.now(),status:{state:"connected",registered:true,updated_at:new Date().toISOString(),spendable_sats:"9007199254740993",pending_sats:null,needs_refresh_sats:"1000",next_refresh_height:null,min_expiry_height:1000},inputs_updated_at_ms:Date.now(),inputs:[{id:vtxo,amount_sats:"1000",state:"spendable",rail:"normal",expiry_height:1000}],payment_wallet:{state:"not_available"}}]});
  if (req.url.includes("/events")) {
    streams++;
    res.writeHead(200,{"content-type":"text/event-stream","cache-control":"no-store"});
    // Real HTTP writes split a multiline resync and UTF-8 payload.
    const frame = Buffer.from(`id: 7\nevent: resync\ndata: ${JSON.stringify(snapshot(user)).slice(0,-1)},\ndata: "test_note":"☃"}\n\n`);
    for (const byte of frame) res.write(Buffer.from([byte]));
    const timer = setInterval(() => { if (revoked) res.end(); else res.write(": heartbeat\n\n"); },100);
    res.on("close",() => {clearInterval(timer);streams--;}); return;
  }
  if (req.url.endsWith("/commands") && req.method === "POST") {
    let body=""; req.on("data",b=>body+=b);req.on("end",()=>{
      const command=JSON.parse(body); commands.push(command);
      json({command_id:command.command_id,state:"unknown",detail:"TEST fixture transport unknown; do not duplicate payment",resolved_at:Date.now(),journal_seq:9});
    });return;
  }
  res.writeHead(404).end();
});

class CDP {
  constructor(url) {
    this.id=0;this.pending=new Map();this.handlers=[];
    this.ws=new WebSocket(url);
    this.ready=new Promise((resolve,reject)=>{this.ws.onopen=resolve;this.ws.onerror=reject;});
    this.ws.onmessage=event=>{const m=JSON.parse(event.data);if(m.id){const p=this.pending.get(m.id);this.pending.delete(m.id);m.error?p.reject(new Error(m.error.message)):p.resolve(m.result);}else for(const h of this.handlers)h(m);};
  }
  async send(method,params={}) {await this.ready;const id=++this.id;return new Promise((resolve,reject)=>{this.pending.set(id,{resolve,reject});this.ws.send(JSON.stringify({id,method,params}));});}
  async eval(expression) {const r=await this.send("Runtime.evaluate",{expression,returnByValue:true,awaitPromise:true});if(r.exceptionDetails)throw new Error(r.exceptionDetails.text);return r.result.value;}
  close(){this.ws.close();}
}
const delay = ms => new Promise(r=>setTimeout(r,ms));

(async()=>{
  let chrome, cdp;
  const checks=[];
  try {
    await new Promise(r=>server.listen(0,"127.0.0.1",r));
    const origin=`http://127.0.0.1:${server.address().port}`;
    const chromePath=process.env.CHROME || "/usr/bin/google-chrome";
    const profile=fs.mkdtempSync(path.join(out,"profile-"));
    chrome=spawn(chromePath,["--headless=new","--no-sandbox","--disable-dev-shm-usage","--disable-background-networking","--remote-debugging-port=0",`--user-data-dir=${profile}`,"about:blank"],{stdio:["ignore","ignore","pipe"]});
    const browserURL=await new Promise((resolve,reject)=>{let text="";const timeout=setTimeout(()=>reject(new Error("Chrome startup timeout")),15000);chrome.stderr.on("data",b=>{text+=b;const m=text.match(/DevTools listening on (ws:\/\/[^\s]+)/);if(m){clearTimeout(timeout);resolve(m[1]);}});chrome.once("error",reject);});
    const browser=new CDP(browserURL);
    const target=await browser.send("Target.createTarget",{url:"about:blank"});
    const port=new URL(browserURL).port;
    const pages=await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    cdp=new CDP(pages.find(p=>p.id===target.targetId).webSocketDebuggerUrl);browser.close();
    cdp.handlers.push(m=>{
      if(m.method==="Runtime.exceptionThrown") exceptions.push(m.params.exceptionDetails.text);
      if(m.method==="Log.entryAdded") logs.push({level:m.params.entry.level,text:m.params.entry.text});
      if(m.method==="Network.loadingFailed" && !m.params.canceled) failures.push(m.params.errorText);
      if(m.method==="Network.responseReceived") responses.push({url:m.params.response.url,status:m.params.response.status});
    });
    await Promise.all([cdp.send("Runtime.enable"),cdp.send("Network.enable"),cdp.send("Log.enable"),cdp.send("Page.enable")]);
    await cdp.send("Emulation.setDeviceMetricsOverride",{width:390,height:844,deviceScaleFactor:1,mobile:true});
    const navigate=async route=>{await cdp.send("Page.navigate",{url:origin+route});await delay(500);};
    const click=async selector=>{const p=await cdp.eval(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.scrollIntoView({block:'center'});const r=e.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2};})()`);await cdp.send("Input.dispatchMouseEvent",{type:"mousePressed",button:"left",clickCount:1,...p});await cdp.send("Input.dispatchMouseEvent",{type:"mouseReleased",button:"left",clickCount:1,...p});};
    const login=async who=>{await cdp.eval(`document.getElementById('auth-token').focus()`);await cdp.send("Input.insertText",{text:token[who]});await click("#auth-form button");await delay(700);};
    await navigate("/lab");
    assert.equal(await cdp.eval("document.getElementById('private-view').hidden"),true);
    assert.equal(await cdp.eval("document.getElementById('auth-form').hidden"),false);checks.push("401 displays login, private DOM empty");
    await login("operator");
    assert.equal(await cdp.eval("document.getElementById('run-id').textContent"),"fixture-run");
    assert.equal(await cdp.eval("!!document.querySelector('#groups img') || !!window.XSS || !!document.querySelector('a[href^=\"javascript:\"]')"),false);
    await cdp.eval("document.querySelector('#groups summary').focus()");
    await cdp.send("Input.dispatchKeyEvent",{type:"keyDown",key:"Enter",code:"Enter",windowsVirtualKeyCode:13});
    await cdp.send("Input.dispatchKeyEvent",{type:"keyUp",key:"Enter",code:"Enter",windowsVirtualKeyCode:13});
    await delay(2300);
    assert.equal(await cdp.eval("document.activeElement.matches('#groups summary')"),true);checks.push("operator directory, hostile markup/URL inert, keyboard focus survives poll");
    assert.equal(await cdp.eval("document.documentElement.scrollWidth <= innerWidth"),true);checks.push("390x844 document has no horizontal overflow");
    fs.writeFileSync(path.join(out,"operator-mobile.png"),Buffer.from((await cdp.send("Page.captureScreenshot")).data,"base64"));
    await click("#logout");await delay(150);
    assert.equal(await cdp.eval("sessionStorage.length"),0);checks.push("logout clears tab credential and private DOM");
    const requestStart=requests.length;
    await login("alice"); // /lab redirects users to the root own view.
    await delay(700);
    assert(!requests.slice(requestStart).some(r=>/\/(inventory|catalog)$/.test(r.url)));
    assert.match(await cdp.eval("document.getElementById('wallet-balance').textContent"),/9,007,199,254,740,993 sats/);
    assert.match(await cdp.eval("document.getElementById('wallet-balance').textContent"),/unknown pending/);checks.push("user never requests Inventory/catalog; exact >2^53 funds and unknown pending");
    await cdp.eval("document.querySelector('input[data-budget]').focus()");await cdp.send("Input.insertText",{text:"42"});
    await delay(2300);
    assert.equal(await cdp.eval("document.activeElement.matches('input[data-budget]') && document.activeElement.value === '42'"),true);
    await click("#wallet-details summary"); await click("#wallet-inputs input");
    await delay(2300);
    assert.equal(await cdp.eval("document.getElementById('wallet-details').open && document.querySelector('#wallet-inputs input').checked"),true);checks.push("worker budget focus/value and wallet disclosure/selection survive polling");
    await click("#wallet-refresh");await delay(250);
    const command=commands.at(-1);assert.equal(command.kind.type,"wallet_refresh");assert.equal(command.actor,"user/alice");assert.deepEqual(command.kind.vtxo_ids,[vtxo]);
    assert.match(await cdp.eval("document.getElementById('command-state').textContent"),/unknown/);
    await click("#retry-command");await delay(250);assert.deepEqual(commands.at(-1),command);checks.push("actual command DTO click; unknown retains exact immutable id/payload on check");
    await navigate("/");await delay(500);
    assert.match(await cdp.eval("document.getElementById('command-state').textContent"),/retained unknown/);
    await click("#retry-command");await delay(250);assert.deepEqual(commands.at(-1),command);checks.push("same-context reload preserves unknown command without a new submission id");
    assert.equal(await cdp.eval("document.documentElement.scrollWidth <= innerWidth"),true);
    assert.equal(await cdp.eval("localStorage.length"),0);
    assert.equal(await cdp.eval("document.getElementById('auth-token').value"),"");
    fs.writeFileSync(path.join(out,"user-mobile.png"),Buffer.from((await cdp.send("Page.captureScreenshot")).data,"base64"));
    revoked=true;await delay(2400);
    assert.equal(await cdp.eval("document.getElementById('private-view').hidden"),true);
    assert.equal(await cdp.eval("sessionStorage.length"),0);assert.equal(streams,0);checks.push("revocation closes real fetch stream and clears private caches");
    token.alice = "c".repeat(64); revoked=false; await login("alice");
    assert.equal(await cdp.eval("document.getElementById('private-view').hidden"),false);
    assert.equal(await cdp.eval("sessionStorage.getItem('spc-lab-token:fixture-run:alice')"),token.alice);checks.push("rotated credential explicitly reauthenticates after old stream closes");
    await click("#logout");assert.equal(await cdp.eval("sessionStorage.length"),0);legacy=true;await navigate("/");await delay(300);
    assert.equal(await cdp.eval("document.getElementById('private-view').hidden"),false);checks.push("legacy off works without token");
    await cdp.send("Emulation.setDeviceMetricsOverride",{width:1440,height:900,deviceScaleFactor:1,mobile:false});
    assert.equal(await cdp.eval("document.documentElement.scrollWidth <= innerWidth"),true);checks.push("1440x900 desktop document has no horizontal overflow");
    fs.writeFileSync(path.join(out,"desktop.png"),Buffer.from((await cdp.send("Page.captureScreenshot")).data,"base64"));
    assert(requests.every(r=>!Object.values(token).some(t=>r.url.includes(t))));assert.equal(exceptions.length,0);assert.equal(failures.length,0);
    checks.push("no credential URLs, persistent storage, uncaught exceptions or unexpected network failures");
    fs.writeFileSync(path.join(out,"proof.json"),JSON.stringify({scope:"TEST-ONLY HTTP and wallet provider fixture; not a compiled controller or funded wallet proof",checks,
      assets:Object.fromEntries(Object.entries(assets).map(([p,b])=>[p,sha(b)])),node_binary:process.execPath,node_sha256:sha(fs.readFileSync(process.execPath)),chrome:chromePath,requests,responses,exceptions,failures,logs},null,2));
    console.log(JSON.stringify({passed:checks.length,out}));
  } finally {
    cdp?.close();
    if(chrome){chrome.kill("SIGTERM");await new Promise(resolve=>{const timer=setTimeout(()=>{chrome.kill("SIGKILL");resolve();},3000);chrome.once("exit",()=>{clearTimeout(timer);resolve();});});}
    server.closeAllConnections();await new Promise(resolve=>server.close(resolve));
  }
})().catch(error=>{fs.writeFileSync(path.join(out,"failure.json"),JSON.stringify({error:error.message,exceptions,failures,logs},null,2));console.error(error);process.exitCode=1;});
