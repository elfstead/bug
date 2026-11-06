#!/usr/bin/env python3
"""Real-engine eval replies, clipboard primitives, metadata and session restore."""
import copy
import json
import os
from pathlib import Path
import queue
import subprocess
import tempfile
import threading
import time

root = Path(__file__).resolve().parents[1]
env = os.environ.copy()
for key in ('DISPLAY', 'WAYLAND_DISPLAY'): env.pop(key, None)
binary = os.environ.get('BUG_TEST_BINARY', str(root/'target/debug/bug'))

class Client:
    def __init__(self, profile, *args):
        self.log = tempfile.TemporaryFile(mode='w+')
        self.child = subprocess.Popen([binary, '--headless', '--profile', str(profile), *args], env=env,
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log, text=True)
        self.events = queue.Queue()
        self.history = []
        def read():
            for line in self.child.stdout:
                try: self.events.put(json.loads(line))
                except json.JSONDecodeError: self.events.put({'event':'invalid-json', 'line':line})
            self.events.put({'event':'eof'})
        threading.Thread(target=read, daemon=True).start()
    def send(self, command):
        self.child.stdin.write(json.dumps(command)+'\n'); self.child.stdin.flush()
    def wait(self, predicate, allow_error=False):
        deadline = time.monotonic()+40
        while time.monotonic()<deadline:
            e = self.events.get(timeout=max(.01, deadline-time.monotonic()))
            self.history.append(e)
            assert e['event'] not in ('eof','invalid-json'), e
            assert allow_error or e['event'] != 'error', e
            if predicate(e): return e
        raise AssertionError('timed out')
    def eval(self, view, script):
        self.send({'cmd':'eval','view':view,'script':script})
        request = self.wait(lambda e:e['event']=='evaluation_queued')['request']
        return self.wait(lambda e:e['event']=='evaluated' and e['request']==request)['result']
    def stop(self):
        if self.child.poll() is None:
            self.send({'cmd':'quit'})
            assert self.child.wait(timeout=15)==0
    def cleanup(self):
        if self.child.poll() is None:
            self.child.terminate()
            try:self.child.wait(timeout=10)
            except subprocess.TimeoutExpired:self.child.kill();self.child.wait()
        self.log.seek(0)
        diagnostics = self.log.read()
        self.log.close()
        return diagnostics

with tempfile.TemporaryDirectory(prefix='bug-features-') as tmp:
    clients = []
    try:
        c = Client(Path(tmp)/'profile', (root/'examples/smoke.html').as_uri()); clients.append(c)
        c.wait(lambda e:e['event']=='ready')
        view = c.wait(lambda e:e['event']=='opened')['view']
        c.wait(lambda e:e['event']=='loaded')
        assert c.eval(view, '21*2') == {'type':'json','value':42}
        assert c.eval(view, "Promise.resolve({title:document.title, items:[1, true, null, '日本語']})") == {'type':'json','value':{'title':'Bug renderer smoke test','items':[1,True,None,'日本語']}}
        for script, result in [('undefined', {'type':'undefined'}), ('NaN', {'type':'number','value':'NaN'}), ('123n', {'type':'bigint','value':'123'})]:
            assert c.eval(view, script)==result
        for script, name in [("throw new Error('bad news')",'Error'), ('const =','SyntaxError'), ("Promise.reject(new TypeError('nope'))",'TypeError'), ('const cyclic={}; cyclic.self=cyclic; cyclic','TypeError')]:
            result = c.eval(view, script)
            assert result['type']=='error' and result['name']==name, result
        # Correlate concurrent requests; the slower first script must not be lost.
        for script in ["new Promise(r=>setTimeout(()=>r('first'),50))", "'second'"]:
            c.send({'cmd':'eval','view':view,'script':script})
        queued = [c.wait(lambda e:e['event']=='evaluation_queued')['request'] for _ in range(2)]
        replies = [c.wait(lambda e:e['event']=='evaluated') for _ in range(2)]
        assert [(e['request'], e['result']['value']) for e in replies] == list(zip(queued,['first','second']))
        # Unicode paste uses Ladybird's edit path, not individual synthetic keys.
        c.eval(view,"document.querySelector('input').focus()")
        c.send({'cmd':'paste','view':view,'text':'clipboard 日本語 🐛'})
        c.wait(lambda e:e['event']=='title' and e['title']=='Bug input: clipboard 日本語 🐛')
        c.eval(view,"document.querySelector('input').select()")
        c.send({'cmd':'copy','view':view})
        assert c.wait(lambda e:e['event']=='clipboard')['text']=='clipboard 日本語 🐛'
        c.send({'cmd':'cut','view':view})
        c.wait(lambda e:e['event']=='clipboard')
        assert c.eval(view,"document.querySelector('input').value") == {'type':'json','value':''}
        metadata = {'name':'work','tags':['pinned'],'properties':{'workspace':'2','purpose':'test'}}
        c.send({'cmd':'set_metadata','view':view,'metadata':metadata})
        assert c.wait(lambda e:e['event']=='metadata')['metadata']==metadata
        c.send({'cmd':'find','name':'work'})
        assert c.wait(lambda e:e['event']=='found')['view']==view
        c.send({'cmd':'resize','view':view,'width':900,'height':700})
        c.send({'cmd':'zoom','view':view,'factor':1.25})
        c.send({'cmd':'set_load_script','view':view,'script':"document.title='restored hook'"})
        c.send({'cmd':'snapshot'})
        snapshot = c.wait(lambda e:e['event']=='session')['session']
        assert snapshot['views'][0]['metadata']==metadata
        assert snapshot['views'][0]['viewport']['width']==900
        assert snapshot['views'][0]['zoom']==1.25
        invalid = copy.deepcopy(snapshot); invalid['views'][0]['zoom']=0
        c.send({'cmd':'restore','session':invalid})
        c.wait(lambda e:e['event']=='error',allow_error=True)
        c.send({'cmd':'restore','session':snapshot})  # existing stable name collision
        c.wait(lambda e:e['event']=='error',allow_error=True)
        c.send({'cmd':'list'})
        assert len(c.wait(lambda e:e['event']=='views')['views'])==1
        session_path = Path(tmp)/'session.json'
        c.send({'cmd':'save_session','path':str(session_path)})
        c.wait(lambda e:e['event']=='session_saved')
        assert json.loads(session_path.read_text())==snapshot
        # Closing an active infinite promise produces an explicit cancellation.
        c.send({'cmd':'open','url':'about:blank'})
        other = c.wait(lambda e:e['event']=='opened')['view']
        c.send({'cmd':'eval','view':other,'script':'new Promise(()=>{})'})
        pending = c.wait(lambda e:e['event']=='evaluation_queued')['request']
        c.send({'cmd':'close','view':other})
        result = c.wait(lambda e:e['event']=='evaluated' and e['request']==pending)['result']
        assert result['type']=='error' and result['name']=='Cancelled', result
        c.stop()
        restored = Client(Path(tmp)/'restored-profile','--session',str(session_path)); clients.append(restored)
        restored.wait(lambda e:e['event']=='title' and e['title']=='restored hook')
        restored.send({'cmd':'list'})
        views = restored.wait(lambda e:e['event']=='views')['views']
        assert len(views)==1 and views[0]['metadata']==metadata and views[0]['zoom']==1.25
        assert views[0]['viewport']==snapshot['views'][0]['viewport']
        assert restored.eval(views[0]['view'], 'document.title') == {'type':'json','value':'restored hook'}
        restored.stop()
        print('PASS: eval values/promises/errors/correlation/cancellation, Unicode copy/cut/paste, named metadata, snapshot validation, save and fresh-process restore')
    except BaseException:
        for c in clients:
            c.log.flush(); c.log.seek(0); print(c.log.read()); print(c.history[-20:])
        raise
    finally:
        for c in clients: c.cleanup()
