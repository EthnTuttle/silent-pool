"""Private v2 Unix source-feed server with durable, signed-window ACKs.

The provider is the independent ReplayProvider, not an input bundle or a
pool-supplied `complete` bit. This service has no miner or wallet operation.
"""
import hashlib
import fcntl
import json
import os
from pathlib import Path
import socket
import stat
import struct
import threading
import time
from concurrent.futures import ThreadPoolExecutor

from case_contract import canonical_json
from oracle import Inconclusive, require, strict_json
from source_feed_codec import (SCHEMA, DEADLINE, MAX_LIFETIME_MS, MAX_FRAME, array, checkpoint_digest,
    consumed_digest, received_digest, trace_digest, verify_signature, encode_frame, receive, send, validate_frame, bundle_window)


class Journal:
    def __init__(self, root):
        self.root=Path(root)
        require(self.root.is_dir() and self.root.resolve()==self.root and self.root.stat().st_uid==os.getuid()
                and self.root.stat().st_mode & 0o077==0,"private collector journal directory")
        self.owner=os.fdopen(os.open(self.root/"collector.lock",os.O_RDWR|os.O_CREAT|os.O_NOFOLLOW,0o600),"a+")
        try:fcntl.flock(self.owner,fcntl.LOCK_EX|fcntl.LOCK_NB)
        except BlockingIOError:
            self.owner.close();raise Inconclusive("collector journal already has a writer")
        self.path=self.root/"feed-journal.jsonl"; self.rows=[]; self.previous="00"*32;self.poisoned=False
        if self.path.exists():
            require(self.path.is_file() and not self.path.is_symlink() and self.path.stat().st_mode & 0o077==0,"collector journal file privacy")
            raw=self.path.read_bytes()
            require(len(raw)<=64*1024*1024,"collector journal bound")
            if raw and not raw.endswith(b"\n"):
                raise Inconclusive("collector journal truncated; no repair or re-authorization")
            for line in raw.splitlines():
                row=strict_json(line); digest=row.pop("digest")
                require(row["seq"]==len(self.rows)+1 and row["previous"]==self.previous,"collector journal prefix/link")
                require(hashlib.sha256(canonical_json(row)).hexdigest()==digest,"collector journal hash")
                self.rows.append(row); self.previous=digest
        fd=os.open(self.path,os.O_WRONLY|os.O_APPEND|os.O_CREAT|os.O_NOFOLLOW,0o600)
        self.stream=os.fdopen(fd,"ab")
        # Re-establish durability of recovered ACK records before any replay
        # can create a previously unissued committed signature.
        try:os.fsync(self.stream.fileno())
        except OSError:
            self.poisoned=True
            try:self.stream.close()
            finally:self.owner.close()
            raise
        self.blob_bytes=sum(p.stat().st_size for p in self.root.glob("*.json"))

    def append(self, kind, value):
        if self.poisoned:raise Inconclusive("collector journal poisoned; no further grants")
        row={"seq":len(self.rows)+1,"previous":self.previous,"kind":kind,"value":value}
        digest=hashlib.sha256(canonical_json(row)).hexdigest()
        try:
            self.stream.write(canonical_json({**row,"digest":digest})+b"\n")
            self.stream.flush(); os.fsync(self.stream.fileno())
        except OSError:
            self.poisoned=True
            raise
        self.rows.append(row); self.previous=digest
        return row["seq"]

    def put_blob(self, value):
        if self.poisoned:raise Inconclusive("collector journal poisoned; no further grants")
        raw=canonical_json(value)
        require(len(raw)<=MAX_FRAME,"collector stored frame bound")
        digest=hashlib.sha256(raw).hexdigest(); path=self.root/(digest+".json")
        if not path.exists():
            require(self.blob_bytes+len(raw)<=512*1024*1024,"collector retained blob budget")
        try:
            fd=os.open(path,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
        except FileExistsError:
            require(not path.is_symlink() and path.read_bytes()==raw,"collector blob changed")
        else:
            with os.fdopen(fd,"wb") as f:f.write(raw);f.flush();os.fsync(f.fileno())
            self.blob_bytes+=len(raw)
            fd=os.open(self.root,os.O_RDONLY|os.O_DIRECTORY)
            try:os.fsync(fd)
            finally:os.close(fd)
        return digest

    def get_blob(self, digest):
        if self.poisoned:raise Inconclusive("collector journal poisoned; no replay grants")
        require(len(digest)==64 and all(c in "0123456789abcdef" for c in digest),"collector blob name")
        path=self.root/(digest+".json")
        require(not path.is_symlink() and path.stat().st_size<=MAX_FRAME,"collector blob type/bound")
        raw=path.read_bytes();require(hashlib.sha256(raw).hexdigest()==digest,"collector stored frame hash")
        return strict_json(raw)

    def close(self):
        try:self.stream.close()
        finally:self.owner.close()


class FeedServer:
    def __init__(self, root, signer, agents, provider, *, now_ms=None):
        from source_feed_provider import ReplayProvider
        require(type(provider) is ReplayProvider,"production collector requires the independent replay provider")
        self.root=Path(root);self.signer=signer;self.agents=agents;self.provider=provider
        self.now_ms=now_ms or (lambda:time.time_ns()//1000000)
        for agent, binding in agents.items():
            require(len({binding["scan_pubkey"],binding["pool_pubkey"],signer.public_key})==3,
                    "collector key must be independently pinned")
        self.journal=Journal(root); self.lock=threading.RLock(); self.attempts={};self.current={}
        authority={"schema":SCHEMA,"collector_pubkey":signer.public_key,"agents":agents}
        if self.journal.rows:
            require(self.journal.rows[0]["kind"]=="authority" and self.journal.rows[0]["value"]==authority,
                    "collector journal belongs to another run/key/agent binding")
        else:self.journal.append("authority",authority)
        # Reconstruct only previously persisted exchanges, never mint a fresh
        # expiry for a reused attempt or forget a later superseding candidate.
        for row in self.journal.rows[1:]:
            value=row["value"]; token=value["attempt"]
            if row["kind"]=="begin":
                require(token not in self.attempts,"collector duplicate begin")
                self.attempts[token]=dict(value);self.current[value["agent"]]=token
            elif row["kind"] in {"prepared","consumed","committed"}:
                require(token in self.attempts,"collector event without attempt")
                self.attempts[token][row["kind"]]=value["blob"]
            else:
                raise Inconclusive("unknown collector journal event")

    def _fresh(self, agent, token, checkpoint):
        require(self.current.get(agent)==token,"source candidate superseded")
        now=self.now_ms()
        if not now < checkpoint["expires_at_unix_ms"] <= now+MAX_LIFETIME_MS:
            raise Inconclusive("source checkpoint expired/lifetime")

    def proof(self, agent, frame, deadline=None):
        deadline = deadline if deadline is not None else time.monotonic()+DEADLINE
        validate_frame(frame)
        require(agent in self.agents and frame["kind"]=="job","source agent/job identity")
        token=array(frame["attempt"],16).hex(); digest=received_digest(frame["job"]).hex()
        with self.lock:
            if token in self.attempts:
                entry=self.attempts[token]
                require(entry["agent"]==agent and entry["job_digest"]==digest,"attempt replay changed agent or job")
                if "prepared" not in entry:
                    raise Inconclusive("prior attempt incomplete; no regenerated proof")
                proof=self.journal.get_blob(entry["prepared"])
                validate_frame(proof)
                self._fresh(agent,token,proof["checkpoint"])
                return proof
            if len(self.attempts)>=4096:raise Inconclusive("collector attempt retention bound")
            entry={"attempt":token,"agent":agent,"job_digest":digest,"created_unix_ms":self.now_ms()}
            self.journal.append("begin",entry)
            self.attempts[token]=entry;self.current[agent]=token
        # May perform bounded read-only collection/replay. A concurrent newer
        # candidate invalidates this result before a checkpoint can be sent.
        evidence=self.provider.prepare(agent,frame["job"])
        if time.monotonic()>=deadline:raise Inconclusive("source evidence missed exchange deadline")
        from source_feed_provider import VerifiedEvidence
        require(isinstance(evidence,VerifiedEvidence),"proof provider must independently verify complete evidence")
        bundle_window(evidence.bundle)
        binding=self.agents[agent]
        verify_signature(trace_digest(evidence.pool),evidence.pool["signature"],binding["pool_pubkey"])
        cp={"attempt":frame["attempt"],"wire_job_digest":list(bytes.fromhex(digest)),
            "job_id":evidence.job_id,"generation":evidence.generation,"source_watermark":evidence.pool["seq"],
            "history_watermark":evidence.history_watermark,"bundle_digest":list(hashlib.sha256(evidence.bundle).digest()),
            "history_digest":list(evidence.history_digest),"expires_at_unix_ms":entry["created_unix_ms"]+MAX_LIFETIME_MS,
            "complete":True,"pool":evidence.pool,"signature":""}
        with self.lock:
            self._fresh(agent,token,cp)
            cp["signature"]=self.signer.checkpoint(cp)
            proof={"kind":"proof","schema":SCHEMA,"bundle_hex":evidence.bundle.hex(),"checkpoint":cp}
            blob=self.journal.put_blob(proof)
            self.journal.append("prepared",{"attempt":token,"blob":blob})
            self.attempts[token]["prepared"]=blob
            return proof

    def consume(self, agent, frame):
        validate_frame(frame)
        require(frame["kind"]=="consumed","consumed frame required")
        ack=frame["ack"];token=array(ack["attempt"],16).hex()
        with self.lock:
            if token not in self.attempts or "prepared" not in self.attempts[token]:
                raise Inconclusive("consumption has no retained prepared proof")
            entry=self.attempts[token];require(entry["agent"]==agent,"consumption agent mismatch")
            prepared=self.journal.get_blob(entry["prepared"])
            require(prepared["schema"]==SCHEMA,"legacy prepared activation frame cannot be reused")
            cp=prepared["checkpoint"]
            self._fresh(agent,token,cp)
            require(all(ack[k]==cp[k] for k in ("attempt","job_id","generation","bundle_digest","history_digest"))
                    and array(ack["checkpoint_digest"],32)==checkpoint_digest(cp),"consumption checkpoint correlation")
            require(ack["window"]==bundle_window(bytes.fromhex(prepared["bundle_hex"])),"wallet ACK window differs from independently verified bundle")
            digest=consumed_digest(ack)
            verify_signature(digest,ack["signature"],self.agents[agent]["scan_pubkey"])
            if "consumed" in entry:
                require(self.journal.get_blob(entry["consumed"])==frame,"consumption replay changed bytes")
            else:
                blob=self.journal.put_blob(frame)
                self.journal.append("consumed",{"attempt":token,"blob":blob})
                entry["consumed"]=blob
            if "committed" in entry:
                return self.journal.get_blob(entry["committed"])
            # The B_scan ACK is fsynced above before this signature is made.
            reply={"kind":"committed","schema":SCHEMA,"attempt":ack["attempt"],"ack_digest":list(digest),
                   "signature":self.signer.committed(ack["attempt"],list(digest))}
            blob=self.journal.put_blob(reply)
            self.journal.append("committed",{"attempt":token,"blob":blob});entry["committed"]=blob
            self._fresh(agent,token,cp)
            return reply

    def exchange(self, stream, agent):
        deadline=time.monotonic()+DEADLINE;frame=None
        try:
            pid,uid,_=struct.unpack("3i",stream.getsockopt(socket.SOL_SOCKET,socket.SO_PEERCRED,12))
            require(uid==os.getuid(),"collector peer UID")
            binding=self.agents[agent]
            from storage import ProcFS
            identity=ProcFS().identity(pid)
            require(pid==binding["pid"] and identity["start_time_ticks"]==binding["start_time_ticks"],"collector peer process identity")
            if "executable_identity" in binding:
                exe=os.stat(f"/proc/{pid}/exe")
                require([exe.st_dev,exe.st_ino,exe.st_size,exe.st_mtime_ns,exe.st_ctime_ns]==binding["executable_identity"],
                        "source verifier executable changed after admission")
            frame=receive(stream,deadline)
            proof=self.proof(agent,frame,deadline);send(stream,proof,deadline)
            ack=receive(stream,deadline)
            require(ack.get("ack",{}).get("attempt")==frame["attempt"],"connection ACK attempt mismatch")
            send(stream,self.consume(agent,ack),deadline)
        except (Exception,) as error:
            if frame is not None and frame.get("kind")=="job":
                try:send(stream,{"kind":"unavailable","schema":SCHEMA,"attempt":frame["attempt"]},deadline)
                except Exception:pass
            return {"status":"INCONCLUSIVE","reason":type(error).__name__}
        finally:
            stream.close()
        return {"status":"COMMITTED_CONSUMPTION_ONLY","agent":agent}

    def serve(self, agent, stop, socket_path=None):
        require(agent in self.agents and "/" not in agent,"collector endpoint agent")
        path=Path(socket_path) if socket_path is not None else self.root/(agent+".sock")
        require(path.parent==self.root,"collector socket directory binding")
        require(not path.exists(),"collector socket must be fresh; no adoption")
        with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as listener:
            listener.bind(str(path));os.chmod(path,0o600);inode=path.stat().st_ino
            listener.listen(4);listener.settimeout(0.2)
            with ThreadPoolExecutor(max_workers=4) as workers:
                slots=threading.BoundedSemaphore(4)
                def handle(stream):
                    try:self.exchange(stream,agent)
                    finally:slots.release()
                try:
                    while not stop.is_set():
                        try:stream,_=listener.accept()
                        except socket.timeout:continue
                        if slots.acquire(blocking=False):workers.submit(handle,stream)
                        else:stream.close()
                finally:
                    if path.exists() and path.stat().st_ino==inode:path.unlink()
