#!/usr/bin/env node
/* CDP on the compiled Rust HTTP/controller. Only source-provider inputs and
 * node/miner adapters are owned fixtures; no Node HTTP stand-in is used. */
"use strict";
const fs = require("node:fs"), path = require("node:path"), crypto = require("node:crypto");
const { spawn } = require("node:child_process"), assert = require("node:assert/strict");
const out = process.env.LIVE_UI_COMPILED_PROOF;
assert(out && process.env.LIVE_UI_SERVER_BIN && process.env.LIVE_UI_ACTOR_BIN && process.env.LIVE_UI_CLI_BIN);
fs.mkdirSync(out,{recursive:true,mode:0o700});
const root=path.resolve(__dirname,"../static"), sha=b=>crypto.createHash("sha256").update(b).digest("hex");
const assets=Object.fromEntries(["index.html","app.js","style.css","lab.html","lab.js","lab.css","access.js"].map(n=>[n,fs.readFileSync(path.join(root,n))]));
const delay=ms=>new Promise(r=>setTimeout(r,ms));
const requests=[],responses=[],exceptions=[],failures=[],logs=[],checks=[];
class CDP {
  constructor(url){this.id=0;this.pending=new Map();this.handlers=[];this.ws=new WebSocket(url);this.ready=new Promise((r,j)=>{this.ws.onopen=r;this.ws.onerror=j;});this.ws.onmessage=e=>{const m=JSON.parse(e.data);if(m.id){const p=this.pending.get(m.id);this.pending.delete(m.id);m.error?p.reject(new Error(m.error.message)):p.resolve(m.result);}else for(const h of this.handlers)h(m);};}
  async send(method,params={}){await this.ready;const id=++this.id;return new Promise((resolve,reject)=>{this.pending.set(id,{resolve,reject});this.ws.send(JSON.stringify({id,method,params}));});}
  async eval(expression){const r=await this.send("Runtime.evaluate",{expression,returnByValue:true,awaitPromise:true});if(r.exceptionDetails)throw new Error(r.exceptionDetails.text);return r.result.value;}
  close(){this.ws.close();}
}
function privateJSON(file,value){fs.writeFileSync(file+".tmp",JSON.stringify(value),{mode:0o600});fs.renameSync(file+".tmp",file);}
(async()=>{
 let server,cliServer,cliURL,chrome,cdp,ready;const ctrl={updated_at:new Date().toISOString()};const assetRoutes=[];
 try{
  privateJSON(path.join(out,"control.json"),ctrl);
  server=spawn(process.env.LIVE_UI_SERVER_BIN,["--ignored","--exact","serve_ui","--nocapture"],{env:{...process.env,SPC_LAB_UI_ROOT:out,SPC_LAB_UI_ACTOR_BIN:process.env.LIVE_UI_ACTOR_BIN},stdio:["ignore","pipe","pipe"]});
  const serverLog=fs.createWriteStream(path.join(out,"rust-server.log"),{mode:0o600});server.stdout.pipe(serverLog);server.stderr.pipe(serverLog);
  await new Promise((resolve,reject)=>{let text="";const timer=setTimeout(()=>reject(new Error("Rust server startup timeout")),30000);server.stdout.on("data",b=>{text+=b;if(text.includes("compiled UI fixture ready")){clearTimeout(timer);resolve();}});server.once("exit",code=>{clearTimeout(timer);reject(new Error(`Rust server exited ${code} before ready`));});});
  ready=JSON.parse(fs.readFileSync(path.join(out,"ready.json")));
  const claimDir=path.join(ready.run_dir,"private/access");
  const claims=fs.readdirSync(claimDir).filter(n=>n.endsWith(".json")).map(n=>({file:path.join(claimDir,n),value:JSON.parse(fs.readFileSync(path.join(claimDir,n)))}));
  const operator=claims.find(c=>c.value.principal==="operator"),alice=claims.find(c=>c.value.principal==="id-alice"),bob=claims.find(c=>c.value.principal==="id-bob");
  const api=`/api/lab/v1/runs/${encodeURIComponent(ready.run_id)}`;
  const authFetch=(route,claim)=>fetch(ready.url+route,{headers:{Authorization:`Bearer ${claim.value.token}`}});
  cliServer=spawn(process.env.LIVE_UI_CLI_BIN,["serve","--run-dir",ready.run_dir,"--listen","127.0.0.1:0","--duration-secs","150"],{stdio:["ignore","pipe","pipe"]});
  const cliLog=fs.createWriteStream(path.join(out,"cli-server.log"),{mode:0o600});cliServer.stdout.pipe(cliLog);cliServer.stderr.pipe(cliLog);
  cliURL=await new Promise((resolve,reject)=>{let text="";const timer=setTimeout(()=>reject(new Error("compiled CLI serve startup timeout")),15000);cliServer.stdout.on("data",b=>{text+=b;const m=text.match(/lab view: (http:\/\/127\.0\.0\.1:\d+)/);if(m){clearTimeout(timer);resolve(m[1]);}});cliServer.once("exit",code=>{clearTimeout(timer);reject(new Error(`CLI serve exited ${code} before ready`));});});
  for(const [serverKind,origin]of [["compiled-cli",cliURL],["compiled-controller-fixture",ready.url]]){
    for(const [name,bytes]of Object.entries(assets)){const route=name==="index.html"?"/":name==="lab.html"?"/lab":"/"+name;const response=await fetch(origin+route);assert.equal(response.status,200);const served=Buffer.from(await response.arrayBuffer());assert.deepEqual(served,bytes);assetRoutes.push({server:serverKind,route,bytes:served.length,sha256:sha(served),exact_equal:true});}
    assert.equal((await fetch(origin+api+"/wallets")).status,401);
    assert.equal((await fetch(origin+api+"/inventory",{headers:{Authorization:`Bearer ${alice.value.token}`}})).status,403);
  }
  chrome=spawn(process.env.CHROME||"/usr/bin/google-chrome",["--headless=new","--no-sandbox","--disable-dev-shm-usage","--disable-background-networking","--remote-debugging-port=0",`--user-data-dir=${path.join(out,"chrome-profile")}`,"about:blank"],{stdio:["ignore","ignore","pipe"]});
  const ws=await new Promise((resolve,reject)=>{let text="";const timer=setTimeout(()=>reject(new Error("Chrome timeout")),15000);chrome.stderr.on("data",b=>{text+=b;const m=text.match(/DevTools listening on (ws:\/\/[^\s]+)/);if(m){clearTimeout(timer);resolve(m[1]);}});chrome.once("error",reject);});
  const browser=new CDP(ws),target=await browser.send("Target.createTarget",{url:"about:blank"});
  const pages=await(await fetch(`http://127.0.0.1:${new URL(ws).port}/json/list`)).json();cdp=new CDP(pages.find(p=>p.id===target.targetId).webSocketDebuggerUrl);browser.close();
  cdp.handlers.push(m=>{if(m.method==="Runtime.exceptionThrown")exceptions.push(m.params.exceptionDetails.text);if(m.method==="Network.requestWillBeSent")requests.push({method:m.params.request.method,url:m.params.request.url});if(m.method==="Network.responseReceived")responses.push({url:m.params.response.url,status:m.params.response.status});if(m.method==="Network.loadingFailed"&&!m.params.canceled)failures.push(m.params.errorText);if(m.method==="Log.entryAdded")logs.push({level:m.params.entry.level,text:m.params.entry.text});});
  await Promise.all(["Runtime.enable","Page.enable","Network.enable","Log.enable"].map(m=>cdp.send(m)));
  await cdp.send("Emulation.setDeviceMetricsOverride",{width:390,height:844,deviceScaleFactor:1,mobile:true});
  const nav=async(url=ready.url+"/")=>{await cdp.send("Page.navigate",{url});await delay(650);};
  const click=async selector=>{const p=await cdp.eval(`(()=>{const e=document.querySelector(${JSON.stringify(selector)});e.scrollIntoView({block:'center'});const r=e.getBoundingClientRect();return{x:r.x+r.width/2,y:r.y+r.height/2};})()`);for(const type of ["mousePressed","mouseReleased"])await cdp.send("Input.dispatchMouseEvent",{type,button:"left",clickCount:1,...p});};
  const login=async claim=>{await cdp.eval("document.getElementById('auth-token').focus()");await cdp.send("Input.insertText",{text:claim.value.token});await click("#auth-form button");await delay(900);};
  await nav(ready.url+"/lab");assert.equal(await cdp.eval("document.getElementById('private-view').hidden"),true);assert.equal(await cdp.eval("document.getElementById('auth-form').hidden"),false);checks.push("actual Rust public static bytes and CLI embedding; 401 login without private DOM");
  await login(operator);assert.equal(await cdp.eval("document.getElementById('run-id').textContent"),ready.run_id);assert.equal((await authFetch(api+"/inventory",operator)).status,200);checks.push("operator authenticated Inventory and role directory");
  await cdp.eval("document.querySelector('#groups summary').focus()");await cdp.send("Input.dispatchKeyEvent",{type:"keyDown",key:"Enter",code:"Enter",windowsVirtualKeyCode:13});await cdp.send("Input.dispatchKeyEvent",{type:"keyUp",key:"Enter",code:"Enter",windowsVirtualKeyCode:13});await delay(2300);assert.equal(await cdp.eval("document.activeElement.matches('#groups summary')"),true);checks.push("real keyboard disclosure focus survives Rust inventory polling");
  assert.equal(await cdp.eval("document.documentElement.scrollWidth<=innerWidth"),true);fs.writeFileSync(path.join(out,"operator-mobile.png"),Buffer.from((await cdp.send("Page.captureScreenshot")).data,"base64"));checks.push("390x844 local table scroll without document overflow");
  await click("#logout");assert.equal(await cdp.eval("sessionStorage.length"),0);assert.equal(await cdp.eval("document.getElementById('groups').textContent"),"");checks.push("logout aborts/clears token and private directory DOM");
  const begin=requests.length;await login(alice);await delay(1500);assert(!requests.slice(begin).some(r=>/\/(inventory|catalog)$/.test(new URL(r.url).pathname)));assert.equal((await authFetch(api+"/inventory",alice)).status,403);
  const own=await(await authFetch(api+"/wallets",alice)).json();assert.deepEqual(own.wallets.map(w=>w.identity_id),["id-alice"]);assert(!JSON.stringify(own).includes("id-bob"));assert(!JSON.stringify(own).includes("onerror"));assert.match(own.wallets[0].inputs_snapshot_sha256,/^[0-9a-f]{64}$/);checks.push("user own-wallet scope before provider serialization; Inventory403; source hash and error allowlist");
  assert.match(await cdp.eval("document.getElementById('wallet-balance').textContent"),/9,007,199,254,740,993 sats/);assert.match(await cdp.eval("document.getElementById('wallet-balance').textContent"),/unknown pending/);assert.equal(await cdp.eval("!!document.querySelector('img')||!!document.querySelector('a[href^=\"javascript:\"]')"),false);checks.push("Rust u64 to exact decimal money, null unknown, hostile provider error not HTML");
  await cdp.eval("document.querySelector('input[data-budget]').focus()");await cdp.send("Input.insertText",{text:"42"});await delay(2300);assert.equal(await cdp.eval("document.activeElement.matches('input[data-budget]')&&document.activeElement.value==='42'"),true);await click("#wallet-details summary");await click("#wallet-inputs input");await delay(2300);assert.equal(await cdp.eval("document.getElementById('wallet-details').open&&document.querySelector('#wallet-inputs input').checked"),true);checks.push("single miner control projection plus input/disclosure/selection persistence");
  await click("#wallet-refresh");await delay(300);assert.match(await cdp.eval("document.getElementById('command-state').textContent"),/running/);
  const journal=()=>fs.readFileSync(path.join(ready.run_dir,"private/commands.jsonl"),"utf8").trim().split("\n").map(JSON.parse);
  const first=journal().filter(r=>r.record==="begin"&&r.command.kind.type==="wallet_refresh").at(-1).command;
  assert.equal(first.actor,alice.value.actor);assert.deepEqual(first.kind.vtxo_ids,["ab".repeat(36)]);const wallet=ready.wallets.find(w=>w.identity_id==="id-alice");const queue=fs.readdirSync(path.join(wallet.datadir,"commands")).filter(n=>n.endsWith(".json"));assert.equal(queue.length,1);const queueBytes=fs.readFileSync(path.join(wallet.datadir,"commands",queue[0]));checks.push("actual controller durable command and AP3-shape private queue; submission is running");
  await click('button[data-action="pause"][data-id="w-alice-1"]');await delay(300);
  assert.match(await cdp.eval("document.getElementById('command-state').textContent"),/succeeded/);
  assert.equal(await cdp.eval(`JSON.parse(sessionStorage.getItem('spc-lab-pending:${ready.run_id}:id-alice')).commands.some(c=>c.command_id===${JSON.stringify(first.command_id)})`),true);
  assert.equal(journal().filter(r=>r.record==="begin"&&r.command.kind.type==="pause_workers").length,1);
  checks.push("W3 worker pause succeeds while running wallet envelope remains independently tracked");
  ctrl.registry_unavailable=true;privateJSON(path.join(out,"control.json"),ctrl);await delay(250);
  const beforeMissing=fs.readFileSync(path.join(ready.run_dir,"private/commands.jsonl"));
  await click("#retry-command");await delay(250);assert.match(await cdp.eval("document.getElementById('command-state').textContent"),/unknown/);await nav();assert.match(await cdp.eval("document.getElementById('command-state').textContent"),/retained unknown/);await click("#retry-command");await delay(250);assert.equal(journal().filter(r=>r.record==="begin"&&r.command.kind.type==="wallet_refresh").length,1);assert.deepEqual(fs.readFileSync(path.join(wallet.datadir,"commands",queue[0])),queueBytes);checks.push("unknown and reload replay retain one immutable journal/queue payload");
  assert.deepEqual(fs.readFileSync(path.join(ready.run_dir,"private/commands.jsonl")),beforeMissing);
  assert.equal(await cdp.eval(`JSON.parse(sessionStorage.getItem('spc-lab-pending:${ready.run_id}:id-alice')).commands[0].command_id`),first.command_id);
  checks.push("W1 registry-loss retry/reload stays unknown with original ID and no journal or queue rewrite");
  ctrl.registry_unavailable=false;
  ctrl.result="noop";ctrl.updated_at=new Date().toISOString();privateJSON(path.join(out,"control.json"),ctrl);await delay(250);await click("#retry-command");await delay(250);assert.match(await cdp.eval("document.getElementById('command-state').textContent"),/rejected_precondition/);assert.equal(await cdp.eval("document.getElementById('retry-command').hidden"),true);checks.push("actual owner result noop maps to rejection and releases UI pending state, not perpetual unknown");
  const before=fs.readFileSync(path.join(ready.run_dir,"private/commands.jsonl"));const attack={...first,actor:bob.value.actor};const denied=await fetch(ready.url+api+"/commands",{method:"POST",headers:{Authorization:`Bearer ${bob.value.token}`,"content-type":"application/json",Origin:ready.url},body:JSON.stringify(attack)});const denial=await denied.json();assert.equal(denial.state,"rejected_precondition");assert.equal(denial.journal_seq,0);assert.deepEqual(fs.readFileSync(path.join(ready.run_dir,"private/commands.jsonl")),before);
  alice.value.token=crypto.randomBytes(32).toString("hex");privateJSON(alice.file,alice.value);await delay(2700);assert.equal(await cdp.eval("document.getElementById('private-view').hidden"),true);assert.equal(await cdp.eval("sessionStorage.length"),0);await login(alice);assert.equal(await cdp.eval("document.getElementById('private-view').hidden"),false);checks.push("cross-wallet replay denied before journal; rotated claim closes old session and explicitly reauthenticates");
  assert.equal(await cdp.eval("localStorage.length"),0);fs.writeFileSync(path.join(out,"user-mobile.png"),Buffer.from((await cdp.send("Page.captureScreenshot")).data,"base64"));await nav(ready.legacy_url+"/");assert.equal(await cdp.eval("document.getElementById('private-view').hidden"),false);await cdp.send("Emulation.setDeviceMetricsOverride",{width:1440,height:900,deviceScaleFactor:1,mobile:false});assert.equal(await cdp.eval("document.documentElement.scrollWidth<=innerWidth"),true);
  await nav(cliURL+"/lab");assert.equal(await cdp.eval("document.getElementById('private-view').hidden"),true);await login(operator);assert.match(await cdp.eval("document.getElementById('view-banner').textContent"),/Recorded-run serve/);assert.equal(await cdp.eval("document.documentElement.scrollWidth<=innerWidth"),true);fs.writeFileSync(path.join(out,"desktop.png"),Buffer.from((await cdp.send("Page.captureScreenshot")).data,"base64"));checks.push("actual legacy tokenless controller plus final compiled spc-lab serve login/routes and desktop layout");
  assert.equal(exceptions.length,0);assert.equal(failures.length,0);assert(requests.every(r=>!/[a-f0-9]{64}/.test(r.url)));assert.equal(checks.length,15);
  fs.writeFileSync(path.join(out,"proof.json"),JSON.stringify({scope:"final compiled spc-lab serve static/auth/read-only routes; live wallet flows through the same production Rust RunEngine/HttpServer/AccessStore/CommandLog/adapters in an opt-in compiled host with explicitly synthetic node/miner/agent providers; no funded workflow",checks,assetRoutes,assets:Object.fromEntries(Object.entries(assets).map(([n,b])=>[n,sha(b)])),binaries:Object.fromEntries(["LIVE_UI_SERVER_BIN","LIVE_UI_ACTOR_BIN","LIVE_UI_CLI_BIN"].map(k=>[k,{path:process.env[k],sha256:sha(fs.readFileSync(process.env[k]))}])),requests,responses,exceptions,failures,logs},null,2));console.log(JSON.stringify({passed:checks.length,out}));
 }finally{
  cdp?.close();if(chrome){chrome.kill("SIGTERM");await new Promise(r=>{const t=setTimeout(()=>{chrome.kill("SIGKILL");r();},3000);chrome.once("exit",()=>{clearTimeout(t);r();});});}
  if(cliServer){cliServer.kill("SIGTERM");await new Promise(r=>{const t=setTimeout(()=>{cliServer.kill("SIGKILL");r();},3000);cliServer.once("exit",()=>{clearTimeout(t);r();});});}
  if(server){fs.writeFileSync(path.join(out,"stop"),"");await new Promise((resolve,reject)=>{const t=setTimeout(()=>{server.kill("SIGTERM");reject(new Error("owned Rust fixture stop timeout"));},15000);server.once("exit",code=>{clearTimeout(t);code===0?resolve():reject(new Error(`Rust fixture exit ${code}`));});});}
 }
})().catch(e=>{fs.writeFileSync(path.join(out,"failure.json"),JSON.stringify({error:e.message,checks,exceptions,failures,logs},null,2));console.error(e.message);process.exitCode=1;});
