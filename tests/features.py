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

def check_document_start(c, tmp):
    # A hook registered with open must precede the very first inline script.
    early = Path(tmp) / 'early.html'
    early.write_text("""<!doctype html><script>
        window.first = window.hookCount || 0;
        window.frameResult = new Promise(resolve => window.addEventListener('message', e => resolve(e.data), {once:true}));
        </script><iframe srcdoc="<script>parent.postMessage(window.hookCount || 0, '*')</script>"></iframe>""")
    hook = 'window.hookCount = (window.hookCount || 0) + 1'
    c.send({'cmd':'open','url':early.as_uri(),'document_start_script':hook})
    hooked = c.wait(lambda e:e['event']=='opened')['view']
    c.wait(lambda e:e['event']=='loaded' and e['view']==hooked)
    assert c.eval(hooked, 'Promise.all([first, frameResult])')['value'] == [1, 0]
    c.send({'cmd':'set_document_start_script','view':hooked,'script':''})
    c.send({'cmd':'reload','view':hooked})
    c.wait(lambda e:e['event']=='loaded' and e['view']==hooked)
    assert c.eval(hooked, 'first')['value'] == 0
    c.send({'cmd':'close','view':hooked})
    c.wait(lambda e:e['event']=='closed' and e['view']==hooked)

def check_native_input(c, view):
    c.eval(view, """const f = document.querySelector('input'); f.value=''; f.focus();
        window.typed = new Promise(resolve => f.addEventListener('input',()=>resolve([f.value,window.trusted]),{once:true}));
        f.addEventListener('keydown',e=>window.trusted=e.isTrusted,{once:true});
        document.body.style.height='5000px'; true""")
    for pressed in (True,False):
        c.send({'cmd':'key','view':view,'name':'X','pressed':pressed,'codepoint':120})
    assert c.eval(view, 'window.typed')['value']==['x',True]
    c.eval(view, "window.scrolled = new Promise(resolve=>addEventListener('scroll',()=>resolve(scrollY),{once:true}));true")
    c.send({'cmd':'mouse','view':view,'kind':'wheel','x':100,'y':100,'dy':160})
    assert c.eval(view,'window.scrolled')['value']>0
    c.eval(view,'scrollTo(0,0)')

def check_request_replies(c, view, setup, upload):
    c.send({'cmd':'set_request_handlers','view':view,'kinds':['alert','confirm','prompt','file_picker','permission','before_unload']})
    for kind, expression, payload, expected in (
        ('alert',"(alert('notice'), 'ok')", {}, 'ok'),
        ('confirm',"confirm('proceed?')", {'accept':True}, True),
        ('prompt',"prompt('label','initial')", {'text':'typed\0text'}, 'typed\0text'),
        ('prompt',"prompt('label','initial')", {'text':''}, ''),
        ('prompt',"prompt('label','initial')", {'text':None}, None),
    ):
        button_script = "const b=document.querySelector('button'); b.style='position:fixed;left:0;top:0'; b.onclick=()=>{window.callbackValue=("+expression+");document.title='answered '+Math.random()};true"
        assert c.eval(view,button_script)['value']
        c.send({'cmd':'click','view':view,'x':16,'y':10})
        req=c.wait(lambda e:e['event']=='request' and e['kind']==kind)
        assert req['view']==view
        if kind=='prompt': assert req['default']=='initial'
        if kind=='confirm':
            c.send({'cmd':'reply_request','view':view,'request':req['request'],'reply':{'kind':'prompt','text':'wrong'}})
            c.wait(lambda e:e['event']=='error',allow_error=True)
        response={'cmd':'reply_request','view':view,'request':req['request'],'reply':{'kind':kind,**payload}}
        c.send(response)
        c.wait(lambda e:e['event']=='title' and e['title'].startswith('answered '))
        assert c.eval(view,'window.callbackValue')=={'type':'json','value':expected}
        c.send(response)
        c.wait(lambda e:e['event']=='error',allow_error=True)
    c.send({'cmd':'eval','view':view,'script':"confirm('will disconnect handler')"})
    req=c.wait(lambda e:e['event']=='request')
    assert c.wait(lambda e:e['event']=='evaluated')['result']['name']=='UserPromptOpened'
    c.send({'cmd':'set_request_handlers','view':view,'kinds':['file_picker']})
    assert c.wait(lambda e:e['event']=='request_cancelled')['request']==req['request']
    assert c.eval(view,setup)['value']
    c.send({'cmd':'click','view':view,'x':20,'y':10})
    req=c.wait(lambda e:e['event']=='request' and e['kind']=='file_picker')
    assert req['accept']==['.txt']
    c.send({'cmd':'reply_request','view':view,'request':req['request'],'reply':{'kind':'file_picker','paths':[str(upload)]}})
    assert c.eval(view,"document.querySelector('#upload').files[0].text()")['value']=='picked by the controller'
    c.eval(view,"document.getElementById('upload').remove()")
    c.eval(view, "document.querySelector('input').value=''")

def check_navigation_callbacks(c):
    from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200)
            self.send_header('Content-Type', 'text/html; charset=utf-8')
            self.end_headers()
            self.wfile.write(b"""<!doctype html><script>window.first=window.early;</script>
                <button style="position:fixed;top:0;left:0"
                onclick="Notification.requestPermission().then(value=>document.title=value)">Ask</button>""")

        def log_message(self, *args):
            pass

    with ThreadingHTTPServer(('127.0.0.1', 0), Handler) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        origin = f'http://127.0.0.1:{server.server_port}'
        try:
            c.send({'cmd':'open', 'url':'about:blank', 'document_start_script':'window.early=23'})
            view = c.wait(lambda e:e['event']=='opened')['view']
            c.send({'cmd':'set_request_handlers', 'view':view, 'kinds':['permission','before_unload']})
            c.send({'cmd':'navigate', 'view':view, 'url':origin+'/'})
            c.wait(lambda e:e['event']=='loaded' and e.get('url')==origin+'/')
            assert c.eval(view, 'first')['value']==23
            c.send({'cmd':'subscribe','events':['loaded','title']})
            c.wait(lambda e:e['event']=='subscribed')
            c.send({'cmd':'set_metadata','view':view,'metadata':{'tags':['filtered']}})
            start=len(c.history)
            c.send({'cmd':'click','view':view,'x':15,'y':10})
            prompt=c.wait(lambda e:e['event']=='request' and e['kind']=='permission')
            assert prompt['name']=='notifications' and prompt['origin']==origin, prompt
            assert all(e['event']!='metadata' for e in c.history[start:])
            c.send({'cmd':'reply_request','view':view,'request':prompt['request'],
                    'reply':{'kind':'permission','decision':'block'}})
            c.wait(lambda e:e['event']=='title' and e['title']=='denied')
            c.eval(view,"onbeforeunload=e=>{e.preventDefault();e.returnValue='stay'};true")
            for leave in (False, True):
                c.send({'cmd':'navigate','view':view,'url':origin+'/next'})
                pending=c.wait(lambda e:e['event']=='request' and e['kind']=='before_unload')
                c.send({'cmd':'reply_request','view':view,'request':pending['request'],
                        'reply':{'kind':'before_unload','leave':leave}})
                if not leave:
                    assert c.eval(view,'location.pathname')['value']=='/'
            c.wait(lambda e:e['event']=='loaded' and e.get('url')==origin+'/next')
            assert c.eval(view,'first')['value']==23
            c.send({'cmd':'subscribe','events':None})
            c.wait(lambda e:e['event']=='subscribed')
            c.send({'cmd':'close','view':view})
            c.wait(lambda e:e['event']=='closed' and e['view']==view)
        finally:
            server.shutdown()
            thread.join()

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
        # A headless controller opens the page's real file input and replies by ID.
        upload = Path(tmp) / 'upload.txt'
        upload.write_text('picked by the controller')
        setup = """const input = document.createElement('input');
            input.id='upload'; input.type='file'; input.accept='.txt';
            input.style='position:fixed;top:0;left:0;z-index:9999';
            input.onchange=async()=>document.title=await input.files[0].text();
            document.body.append(input); true"""
        assert c.eval(view, setup) == {'type':'json','value':True}
        c.send({'cmd':'click','view':view,'x':20,'y':10})
        picker = c.wait(lambda e:e['event']=='file_picker_requested')
        assert picker['view']==view and not picker['multiple'] and picker['accept']==['.txt'], picker
        c.send({'cmd':'reply_file_picker','view':view,'request':picker['request'],'paths':[str(upload)]})
        c.wait(lambda e:e['event']=='title' and e['title']=='picked by the controller')
        c.eval(view, "document.getElementById('upload').remove()")
        check_document_start(c, tmp)
        check_native_input(c, view)
        check_request_replies(c, view, setup, upload)
        check_navigation_callbacks(c)
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
        c.send({'cmd':'set_document_start_script','view':view,'script':'window.startRestored=17'})
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
        assert restored.eval(views[0]['view'], 'window.startRestored')['value']==17
        # Closing the command pipe must release a waiting page while the host stays up.
        restored_view=views[0]['view']
        restored.send({'cmd':'set_request_handlers','view':restored_view,'kinds':['confirm']})
        restored.send({'cmd':'eval','view':restored_view,'script':"document.title='fallback '+confirm('stdin closed')"})
        waiting=restored.wait(lambda e:e['event']=='request' and e['kind']=='confirm')
        restored.child.stdin.close()
        assert restored.wait(lambda e:e['event']=='request_cancelled')['request']==waiting['request']
        restored.wait(lambda e:e['event']=='title' and e['title']=='fallback false')
        assert restored.child.poll() is None
        print('PASS: native input, controller requests and replies, document-start and load scripts, file upload, evaluation, clipboard, metadata and session restore')
    except BaseException:
        for c in clients:
            c.log.flush(); c.log.seek(0); print(c.log.read()); print(c.history[-20:])
        raise
    finally:
        for c in clients: c.cleanup()
