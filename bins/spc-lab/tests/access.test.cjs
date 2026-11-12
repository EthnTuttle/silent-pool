"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { SSEParser, AccessSession, sats, mountAccess, PendingCommands } = require("../static/access.js");
const { paymentCommand, walletCommand, trafficCommand } = require("../static/app.js");
const http = require("node:http");

function storage() {
  const values = {};
  Object.defineProperties(values, {
    getItem: { value: k => values[k] ?? null },
    setItem: { value: (k,v) => { values[k] = v; } },
    removeItem: { value: k => { delete values[k]; } },
  });
  return values;
}

test("SSE parses every boundary, CRLF, multiline, persistent id, resync and ignores incomplete EOF", () => {
  const text = ': heartbeat\r\nid: 9007199254740993\r\nevent: resync\r\ndata: {"a":\r\ndata: "snow ☃"}\r\n\r\ndata: tail\n\nid: bad\0id\ndata: final\n\ndata: incomplete';
  for (let split = 0; split <= text.length; split++) {
    const events = [], parser = new SSEParser(e => events.push(e));
    parser.push(text.slice(0,split)); parser.push(text.slice(split));
    assert.deepEqual(events, [
      { type:"resync", id:"9007199254740993", data:'{"a":\n"snow ☃"}' },
      { type:"message", id:"9007199254740993", data:"tail" },
      { type:"message", id:"9007199254740993", data:"final" },
    ]);
  }
  assert.throws(() => new SSEParser(() => {}, 32).push("data: x\n".repeat(10)), /too large/);
});

test("money stays exact and unknown stays unknown", () => {
  assert.equal(sats("18446744073709551615"), "18,446,744,073,709,551,615 sats");
  assert.equal(sats("0"), "0 sats");
  for (const value of [null, undefined, 9007199254740993, "1.2", "-1", "01"]) assert.equal(sats(value), "unknown");
});

test("wallet and payment commands match actual owner DTOs and reject ambiguous values", () => {
  const id = "ab".repeat(36);
  assert.deepEqual(walletCommand("alice", "refresh", [id]), {type:"wallet_refresh",identity_id:"alice",vtxo_ids:[id]});
  assert.throws(() => walletCommand("alice", "refresh", [id,id]));
  assert.throws(() => walletCommand("alice", "offboard", [id], "javascript:alert(1)"));
  const addr = "mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn";
  assert.deepEqual(paymentCommand("alice", addr, "1000", "2"), {type:"send_user_payment",user:"alice",destination_address:addr,amount_sats:1000,fee_rate_sats_vb:2});
  for (const n of ["9007199254740993", "1e3", "1.5", "0", "-1"]) assert.throws(() => paymentCommand("alice",addr,n,"2"));
  assert.deepEqual(trafficCommand("txd-0", "pause"), {type:"pause_tx_driver",driver_id:"txd-0"});
  assert.deepEqual(trafficCommand("txd-0", "resume"), {type:"resume_tx_driver",driver_id:"txd-0"});
});

test("real fetch stream with Bearer: split UTF8/resync, revocation clears session, no credential URL/body", async t => {
  const token = "a".repeat(64), store = storage(), requests = [];
  let revoked = false;
  const server = http.createServer((req,res) => {
    requests.push({url:req.url,auth:req.headers.authorization,cursor:req.headers["last-event-id"]});
    if (revoked || req.headers.authorization !== `Bearer ${token}`) { res.writeHead(401).end(); return; }
    if (req.url.endsWith("health")) return res.end(JSON.stringify({run_id:"r1"}));
    if (req.url.endsWith("me")) return res.end(JSON.stringify({run_id:"r1",principal:"alice",actor:"user/alice",may:["view_own_status"]}));
    res.writeHead(200,{"content-type":"text/event-stream"});
    const bytes = Buffer.from('id: 8\nevent: resync\ndata: {"s":\ndata: "☃"}\n\n');
    for (const byte of bytes) res.write(Buffer.from([byte]));
    res.end();
  });
  await new Promise(r => server.listen(0,"127.0.0.1",r));
  t.after(() => server.close());
  const session = new AccessSession({storage:store,fetcher:(path,init) => fetch(`http://127.0.0.1:${server.address().port}${path}`,init)});
  await session.login(token);
  assert.equal(store["spc-lab-token:r1:alice"],token);
  const events = [];
  await session.stream("/api/lab/v1/runs/r1/events", "7", e => events.push(e));
  assert.equal(events[0].id,"8"); assert.equal(events[0].data,'{"s":\n"☃"}');
  assert.equal(requests.at(-1).cursor,"7");
  revoked = true;
  await assert.rejects(session.json("/api/lab/v1/health"), /authentication/);
  assert.equal(session.context,null); assert.deepEqual(Object.keys(store),[]);
  assert(requests.every(r => !r.url.includes(token)));
});

test("logout aborts streams and pending body reads; run switch drops stored credential", async () => {
  const store = storage(); let signal;
  const session = new AccessSession({ storage:store, fetcher: async (path,init) => {
    signal = init.signal;
    if (path.endsWith("health")) return Response.json({run_id:"r2"});
    return Response.json({run_id:"r2",principal:"bob",actor:"user/bob",may:[]});
  }});
  await assert.rejects(session.login("b".repeat(64),{run_id:"r1",principal:"alice"}),/changed/);
  assert.deepEqual(Object.keys(store),[]); assert.equal(session.token,"");
  const request = await session.request("/api/lab/v1/health");
  session.clear(); assert(signal.aborted); assert.equal(request.controller.signal.aborted,true);
});

test("legacy off discovers without token; 403 never falls back to another read", async () => {
  const paths = [];
  const session = new AccessSession({storage:storage(),fetcher: async (path,init) => {
    paths.push(path); assert.equal(init.headers.get("Authorization"),null);
    return path.endsWith("health") ? Response.json({run_id:"legacy"}) : new Response("secret-other-user",{status:403});
  }});
  assert.equal((await session.restore()).legacy,true);
  await assert.rejects(session.json("/api/lab/v1/runs/legacy/inventory"),/outside your access scope/);
  assert.deepEqual(paths,["/api/lab/v1/health","/api/lab/v1/runs/legacy/inventory"]);
});

test("same-context reload retains unknown payload; explicit switch/logout removes it", async () => {
  const store = storage();
  const session = new AccessSession({storage:store,fetcher:async path => Response.json(path.endsWith("health")
    ? {run_id:"r1"} : {run_id:"r1",principal:"alice",actor:"user/alice",may:[]})});
  await session.login("c".repeat(64));
  const command = {run_id:"r1",actor:"user/alice",command_id:"immutable-id",kind:{type:"wallet_refresh",vtxo_ids:["ab".repeat(36)]}};
  session.savePending(command);
  await session.restore();
  assert.deepEqual(session.loadPending(),command);
  await session.login("c".repeat(64));
  assert.equal(session.loadPending(),null);
  session.savePending(command);session.clear();assert.equal(Object.keys(store).length,0);
});

test("a delayed old-session body cannot repaint after a new token context", async () => {
  let release;
  const session = new AccessSession({storage:storage(),fetcher:async () => new Response(new ReadableStream({start(controller) {
    release = () => {controller.enqueue(new TextEncoder().encode('{"private":"old-principal"}'));controller.close();};
  }}))});
  const pending = session.json("/api/lab/v1/runs/old/snapshot");
  await new Promise(resolve => setImmediate(resolve));
  session.clear();session.context={run_id:"new",principal:"new"};session.token="d".repeat(64);
  release();
  await assert.rejects(pending,/session changed/);
  assert.equal(session.context.principal,"new");
});

test("cancelled bootstrap discovery cannot cancel a newer token login", async () => {
  const originals = Object.fromEntries(["fetch","sessionStorage","document","window"].map(k=>[k,Object.getOwnPropertyDescriptor(globalThis,k)]));
  const nodes = Object.fromEntries(["auth-form","auth-token","private-view","logout","auth-message"].map(id=>[id,{hidden:false,value:"",textContent:"",handlers:{},addEventListener(k,f){this.handlers[k]=f;},focus(){}}]));
  let finishOld, finishNew;
  const calls=[],ready=[];
  try {
    Object.defineProperty(globalThis,"sessionStorage",{configurable:true,value:storage()});
    globalThis.document={getElementById:id=>nodes[id]};globalThis.window={addEventListener(){}};
    globalThis.fetch=async(path,init)=>{
      calls.push(path);
      if(calls.length===1)return new Promise(resolve=>{finishOld=()=>resolve(Response.json({run_id:"old"}));});
      if(path.endsWith("health"))return new Promise(resolve=>{finishNew=()=>resolve(Response.json({run_id:"new"}));});
      return Response.json({run_id:"new",principal:"alice",actor:"user/alice",may:[]});
    };
    const session=mountAccess((_,context)=>ready.push(context.principal),()=>{});
    nodes["auth-token"].value="e".repeat(64);
    const submit=nodes["auth-form"].handlers.submit({preventDefault(){}});
    finishOld();await new Promise(resolve=>setImmediate(resolve));
    finishNew();await submit;
    assert.deepEqual(ready,["alice"]);assert.equal(session.context.principal,"alice");
    assert.equal(nodes["private-view"].hidden,false);
  } finally {
    for(const[k,d]of Object.entries(originals)){if(d)Object.defineProperty(globalThis,k,d);else delete globalThis[k];}
  }
});

test("running/unknown funds retain their envelope while unrelated workers resolve", async () => {
  const wallet={command_id:"wallet-original",run_id:"r1",actor:"user/alice",kind:{type:"wallet_refresh"}};
  const worker={command_id:"worker-original",run_id:"r1",actor:"user/alice",kind:{type:"pause_workers"}};
  const book=new PendingCommands();book.add(wallet);
  assert.equal(book.resolve(wallet,{state:"running",journal_seq:2},false),true);
  book.add(worker);book.resolve(worker,{state:"succeeded",journal_seq:4},false);
  assert.deepEqual(book.snapshot().commands,[wallet]);
  const session=new AccessSession({storage:storage(),fetcher:async path=>Response.json(path.endsWith('health')?{run_id:'r1'}:{run_id:'r1',principal:'alice',actor:'user/alice',may:[]})});
  await session.login('f'.repeat(64));session.savePending(book.snapshot());await session.restore();
  const restored=new PendingCommands(session.loadPending());
  assert.deepEqual(restored.snapshot().commands,[wallet]);
  assert.equal(restored.resolve(wallet,{state:"rejected_precondition",journal_seq:0},true),true);
  assert.throws(()=>restored.add({...wallet,command_id:"duplicate-funds"}),/retained funds/);
  restored.add(worker);
  restored.resolve(wallet,{state:"rejected_precondition",journal_seq:8},true);
  assert.deepEqual(restored.snapshot().commands,[worker]);
  restored.resolve(worker,{state:"unknown",journal_seq:4},true);
  assert.deepEqual(restored.snapshot().commands,[worker]);
});
