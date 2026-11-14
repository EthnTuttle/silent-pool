"""Deterministic writer schedules for the live-prefix correction, not live runs."""
from contextlib import contextmanager
import hashlib
import os
from pathlib import Path
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from campaign import load_controller
from live_journal import LiveJournalPrefix
from oracle import Invalid, Inconclusive
from runtime_bootstrap import RuntimeBootstrap
from test_runtime_bootstrap import fixture, encoded, finish


class LiveJournalTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(prefix='live-prefix-');self.addCleanup(self.tmp.cleanup)
        self.root=Path(self.tmp.name);(self.root/'private').mkdir(mode=0o700)
        self.path=self.root/'private/commands.jsonl';self.scenario,self.rows=fixture()
        self.raw=encoded(self.rows);self.write(self.raw)
        self.next=encoded([{'record':'hash_reconcile','seq':4,'operation_id':'original-phase-id',
            'driver_incarnation':'real-format-unit-incarnation','batch_id':1,'measured_attempts':100}])

    def write(self,raw):
        self.path.write_bytes(raw);self.path.chmod(0o600)

    def reader(self,**kwargs):return LiveJournalPrefix(self.path,self.root,**kwargs)

    def controller(self):
        c=object.__new__(load_controller().Controller);c.run=self.root;c.run_id='run'
        c.phase_bootstrap=RuntimeBootstrap(self.scenario);c.observation_lock=threading.RLock();c.live_journal_readers={}
        self.captures=[];c.capture=SimpleNamespace(add=lambda kind,value:self.captures.append((kind,value)))
        return c

    @contextmanager
    def during_read(self,mutation,*,before_capture=False):
        # A real writer thread runs after the first captured pread and before
        # verification. No size/mtime values or journal bytes are mocked.
        ready=threading.Event();done=threading.Event();errors=[];fired=[]
        def writer():
            try:
                if not ready.wait(5):raise AssertionError('reader never reached scheduled boundary')
                mutation()
            except BaseException as error:errors.append(error)
            finally:done.set()
        thread=threading.Thread(target=writer);thread.start();original=os.pread
        def pread(fd,n,offset):
            raw=None if before_capture else original(fd,n,offset)
            if not fired:
                fired.append(True);ready.set()
                if not done.wait(5):raise AssertionError('scheduled writer stalled')
                if errors:raise errors[0]
            return original(fd,n,offset) if before_capture else raw
        try:
            with patch('live_journal.os.pread',side_effect=pread):yield
        finally:
            ready.set();thread.join(5);self.assertFalse(thread.is_alive())
            if errors:raise errors[0]

    def append(self,raw):
        with self.path.open('ab') as f:f.write(raw);f.flush();os.fsync(f.fileno())

    def test_append_during_snapshot_preserves_original_operation_and_unresolved_reservation(self):
        c=self.controller()
        with self.during_read(lambda:self.append(self.next)):first=c.bootstrap_status()
        self.assertEqual(first['command_prefix_sha256'],hashlib.sha256(self.raw).hexdigest())
        self.assertEqual(first['debits'],{'charged_reserved':100,'measured_executed':0,'charged_unresolved':100})
        later=c.bootstrap_status()
        self.assertEqual(later['debits'],{'charged_reserved':100,'measured_executed':100,'charged_unresolved':0})
        for key in ('operation_id','actor_command_id','principal','budget'):self.assertEqual(later[key],first[key])
        snapshots=[value for kind,value in self.captures if kind=='live_journal_prefix']
        self.assertGreater(snapshots[0]['observed_size_floor'],snapshots[0]['snapshot_bytes'])

    def test_incomplete_tail_is_deferred_without_synthesizing_reconciliation(self):
        c=self.controller();first=c.bootstrap_status();self.append(self.next[:-1]);pending=c.bootstrap_status()
        self.assertEqual(pending,first)
        self.assertEqual(c.live_journal_readers['commands.jsonl'].observation['pending_tail_bytes'],len(self.next)-1)
        self.append(b'\n');self.assertEqual(c.bootstrap_status()['debits']['measured_executed'],100)

    def test_append_after_size_snapshot_before_read_uses_only_captured_length(self):
        c=self.controller()
        with self.during_read(lambda:self.append(self.next),before_capture=True):result=c.bootstrap_status()
        self.assertEqual(result['command_prefix_bytes'],len(self.raw))
        self.assertEqual(result['debits']['charged_unresolved'],100)
        self.assertEqual(c.bootstrap_status()['debits']['charged_unresolved'],0)

    def test_never_created_or_empty_begin_can_wait_but_observed_deletion_is_fatal(self):
        self.path.unlink();r=self.reader()
        with self.assertRaises(FileNotFoundError):r.read()
        self.write(b'');self.assertEqual(r.read(),b'')
        self.path.unlink()
        with self.assertRaisesRegex(Invalid,'disappeared'):r.read()
        self.write(self.raw)
        with self.assertRaisesRegex(Invalid,'poisoned'):r.read()

    def test_committed_or_pending_observed_bytes_cannot_be_rewritten(self):
        for pending in (False,True):
            with self.subTest(pending=pending):
                self.write(self.raw+(self.next[:-1] if pending else b''));r=self.reader();r.read()
                raw=self.path.read_bytes();index=len(self.raw)+1 if pending else 1
                self.write(raw[:index]+b'X'+raw[index+1:])
                with self.assertRaisesRegex(Invalid,'rewritten'):r.read()

    def test_shrink_below_observed_size_including_unread_concurrent_growth_refuses(self):
        r=self.reader()
        with self.during_read(lambda:self.append(self.next)):r.read()
        self.write(self.raw)
        with self.assertRaisesRegex(Invalid,'size regressed'):r.read()

    def test_same_bytes_in_a_replacement_inode_are_not_the_same_journal(self):
        r=self.reader();r.read();other=self.path.with_name('replacement');other.write_bytes(self.raw);other.chmod(0o600)
        os.replace(other,self.path)
        with self.assertRaisesRegex(Invalid,'identity changed'):r.read()

    def test_parent_directory_replacement_and_symlink_are_refused(self):
        r=self.reader();r.read();private=self.root/'private';private.rename(self.root/'old-private')
        private.mkdir(mode=0o700);self.write(self.raw)
        with self.assertRaisesRegex(Invalid,'identity changed'):r.read()
        self.path.unlink();self.path.symlink_to(self.root/'old-private/commands.jsonl')
        with self.assertRaises(OSError):self.reader().read()

    def test_hardlink_mode_fifo_and_foreign_uid_refuse(self):
        alias=self.path.with_name('alias');os.link(self.path,alias)
        with self.assertRaisesRegex(Invalid,'ownership/mode/type/link'):self.reader().read()
        alias.unlink();self.path.chmod(0o644)
        with self.assertRaises(Invalid):self.reader().read()
        self.path.unlink();os.mkfifo(self.path,0o600)
        with self.assertRaises(Invalid):self.reader().read()
        self.path.unlink();self.write(self.raw);original=os.fstat
        def foreign(fd):
            value=original(fd)
            if value.st_ino==self.path.stat().st_ino:
                fields={name:getattr(value,name) for name in ('st_dev','st_ino','st_uid','st_mode','st_nlink','st_size')}
                return SimpleNamespace(**{**fields,'st_uid':os.getuid()+1})
            return value
        with patch('live_journal.os.fstat',side_effect=foreign),self.assertRaises(Invalid):self.reader().read()

    def test_rewrite_or_replacement_during_verification_is_rejected(self):
        def rewrite():
            with self.path.open('r+b') as f:f.write(b'X');f.flush();os.fsync(f.fileno())
        with self.during_read(rewrite),self.assertRaisesRegex(Invalid,'captured prefix changed'):self.reader().read()
        self.write(self.raw)
        def replace():
            other=self.path.with_name('new');other.write_bytes(self.raw);other.chmod(0o600);os.replace(other,self.path)
        with self.during_read(replace),self.assertRaisesRegex(Invalid,'path changed'):self.reader().read()

    def test_record_total_and_concurrent_growth_bounds_are_enforced(self):
        with self.assertRaises(Invalid):self.reader(maximum=len(self.raw)-1).read()
        with self.during_read(lambda:self.append(self.next)),self.assertRaises(Invalid):self.reader(maximum=len(self.raw)).read()
        self.write(b'abc\n'+b'x'*9)
        with self.assertRaisesRegex(Invalid,'record bound'):self.reader(maximum_record=8).read()
        self.write(b'x'*9+b'\n')
        with self.assertRaisesRegex(Invalid,'record bound'):self.reader(maximum_record=8).read()

    def test_path_replacement_after_byte_verification_cannot_hide_in_old_open_fds(self):
        original=os.pread;calls=[]
        def replace_after_second_read(fd,n,offset):
            raw=original(fd,n,offset);calls.append(offset)
            if len(calls)==2:
                self.path.rename(self.path.with_name('retained-old-inode'));self.write(self.raw)
            return raw
        with patch('live_journal.os.pread',side_effect=replace_after_second_read),self.assertRaisesRegex(Invalid,'final path identity'):
            self.reader().read()

    def test_complete_record_corruption_and_budget_reset_still_escape_as_errors(self):
        for raw in (b'{invalid}\n',encoded([{**self.rows[1],'seq':4}])):
            with self.subTest(raw=raw):
                self.write(self.raw);c=self.controller();c.bootstrap_status();self.append(raw)
                with self.assertRaises((Invalid,Inconclusive)):c.bootstrap_status()

    def test_pending_evidence_waits_for_real_phase_record_and_never_invents_terminal(self):
        c=self.controller();c.bootstrap_status();evidence=finish(self.rows);self.write(encoded(self.rows))
        path=self.root/'private/evidence.jsonl';path.write_bytes(evidence[:-1]);path.chmod(0o600)
        with self.assertRaises(Inconclusive):c.bootstrap_status(require_phase=True)
        with path.open('ab') as f:f.write(b'\n')
        result=c.bootstrap_status(require_phase=True);self.assertEqual(result['state'],'succeeded');self.assertIsNotNone(result['phase'])

    def test_strict_complete_option_does_not_promote_a_pending_tail(self):
        self.append(self.next[:-1])
        with self.assertRaisesRegex(Inconclusive,'pending tail'):self.reader().read(require_complete=True)

    def test_concurrent_observers_serialize_without_reordering_prefixes(self):
        c=self.controller();results=[];errors=[]
        def observe():
            try:results.append(c.bootstrap_status())
            except BaseException as error:errors.append(error)
        with self.during_read(lambda:self.append(self.next)):
            threads=[threading.Thread(target=observe) for _ in range(2)]
            for t in threads:t.start()
            for t in threads:t.join(5);self.assertFalse(t.is_alive())
        self.assertEqual(errors,[]);self.assertEqual(sorted(r['debits']['measured_executed'] for r in results),[0,100])

    def test_root_escape_refuses_and_no_open_journal_fd_is_retained(self):
        with self.assertRaises(Invalid):LiveJournalPrefix(self.root.parent/'outside',self.root)
        r=self.reader();r.read()
        def refs():
            found=[]
            for path in Path('/proc/self/fd').iterdir():
                try:
                    if os.readlink(path)==str(self.path):found.append(path.name)
                except FileNotFoundError:pass
            return found
        self.assertEqual(refs(),[]);self.path.chmod(0o644)
        with self.assertRaises(Invalid):r.read()
        self.assertEqual(refs(),[])


if __name__=='__main__':unittest.main()
