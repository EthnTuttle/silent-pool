"""Read verified complete prefixes while the owned append-only writer advances.

This never locks a writer, repairs a tail, or certifies EOF/durability. Immutable
configs and keys must continue using storage.private_bytes. Final capture still
requires the existing writer-stop/EOF/SQL seal and exact post-drain audit.
"""
import hashlib
import os
from pathlib import Path
import stat
import threading

from oracle import Inconclusive, Invalid, require


class LiveJournalPrefix:
    def __init__(self,path,root,maximum=128*1024*1024,maximum_record=16*1024*1024):
        self.path=Path(path);self.root=Path(root)
        require(self.path.is_absolute() and self.root.is_absolute() and '..' not in self.path.parts
            and '..' not in self.root.parts and self.path!=self.root and self.path.is_relative_to(self.root),
            'live journal root confinement')
        require(type(maximum) is int and maximum>0 and type(maximum_record) is int and maximum_record>0,
            'live journal byte limits')
        self.maximum=maximum;self.maximum_record=maximum_record
        self.identity=None;self.seen=b'';self.size_floor=0;self.failed=False
        self.observation=None;self.lock=threading.Lock()

    @staticmethod
    def _close(fds):
        for fd in reversed(fds):os.close(fd)

    def _open(self):
        """Walk each path component with dirfd + NOFOLLOW, retaining the chain."""
        fds=[];chain=[]
        try:
            fds.append(os.open('/',os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW))
            parts=self.path.parts
            for index,name in enumerate(parts[1:-1],1):
                fds.append(os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW,dir_fd=fds[-1]))
                info=os.fstat(fds[-1])
                if index>=len(self.root.parts)-1:
                    require(info.st_uid==os.getuid(),'live journal directory ownership')
                    chain.append((info.st_dev,info.st_ino,info.st_uid,stat.S_IMODE(info.st_mode)))
            fd=os.open(parts[-1],os.O_RDONLY|os.O_NOFOLLOW|os.O_NONBLOCK,dir_fd=fds[-1]);fds.append(fd)
            info=os.fstat(fd);self._metadata(info)
            return fds,(tuple(chain),info.st_dev,info.st_ino,info.st_uid),info
        except BaseException:
            self._close(fds);raise

    def _metadata(self,info):
        require(stat.S_ISREG(info.st_mode) and info.st_uid==os.getuid()
            and stat.S_IMODE(info.st_mode)==0o600 and info.st_nlink==1,
            'live journal file ownership/mode/type/link')
        require(self.size_floor<=info.st_size<=self.maximum,'live journal size regressed or exceeds bound')

    @staticmethod
    def _chunks(fd,size):
        offset=0
        while offset<size:
            raw=os.pread(fd,min(1024*1024,size-offset),offset)
            require(bool(raw),'live journal short read/truncation')
            yield offset,raw;offset+=len(raw)

    def read(self,*,require_complete=False):
        with self.lock:
            require(not self.failed,'live journal reader poisoned by prior failure')
            try:return self._read(require_complete)
            except FileNotFoundError:
                # A never-created log may be awaited by the existing explicit
                # bootstrap-begin loop. A previously observed log cannot vanish.
                if self.identity is None:raise
                self.failed=True;raise Invalid('live journal observed path disappeared') from None
            except Exception:
                self.failed=True;raise

    def _read(self,require_complete):
        fds,identity,before=self._open()
        try:
            if self.identity is not None:require(identity==self.identity,'live journal root/file identity changed')
            size=before.st_size
            raw=b''.join(chunk for _,chunk in self._chunks(fds[-1],size))
            require(raw.startswith(self.seen),'live journal observed prefix rewritten')
            # Reopen through the name as well: an unlinked/replaced inode or a
            # changed parent directory must not be hidden by the first open fd.
            verify,again,middle=self._open()
            try:
                require(again==identity,'live journal path changed during observation')
                require(middle.st_size>=size,'live journal truncated during observation')
                view=memoryview(raw)
                for offset,chunk in self._chunks(verify[-1],size):
                    require(view[offset:offset+len(chunk)]==chunk,'live journal captured prefix changed during read')
                after=os.fstat(verify[-1]);self._metadata(after)
                require(after.st_size>=middle.st_size,'live journal truncated during verification')
                first_after=os.fstat(fds[-1]);self._metadata(first_after)
                require(first_after.st_size>=after.st_size,'live journal final size regression')
            finally:self._close(verify)
            named,last_identity,last=self._open()
            try:
                require(last_identity==identity and last.st_size>=first_after.st_size,
                    'live journal final path identity/size changed')
            finally:self._close(named)
            # Complete records only. Remember the uncommitted tail too, so a
            # later read cannot silently replace even previously seen fragments.
            end=raw.rfind(b'\n')+1;pending=size-end
            require(pending<=self.maximum_record and all(len(line)<=self.maximum_record for line in raw[:end].splitlines()),
                'live journal record bound')
            if require_complete and pending:raise Inconclusive('live journal pending tail is not a complete final capture')
            self.identity=identity;self.seen=raw;self.size_floor=max(size,middle.st_size,after.st_size,first_after.st_size,last.st_size)
            prefix=raw[:end]
            self.observation={'path':str(self.path),'device':identity[1],'inode':identity[2],'uid':identity[3],
                'snapshot_bytes':size,'complete_prefix_bytes':end,'pending_tail_bytes':pending,
                'observed_size_floor':self.size_floor,'complete_prefix_sha256':hashlib.sha256(prefix).hexdigest(),
                'scope':'verified_complete_live_prefix_not_final_EOF'}
            return prefix
        finally:self._close(fds)
