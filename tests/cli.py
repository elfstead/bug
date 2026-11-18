#!/usr/bin/env python3
"""Drive the single-command client against an isolated real browser."""
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import tempfile
import time
import threading

root = Path(__file__).resolve().parents[1]
binary = Path(os.environ.get('BUG_TEST_BINARY', root / 'target/debug/bug')).resolve()
fixture = root / 'examples/smoke.html'
with tempfile.TemporaryDirectory(prefix='bug-cli-') as tmp:
    # Long profile paths must also work on Darwin's shorter AF_UNIX addresses.
    profile = Path(tmp) / ('long profile ' * 9)
    command = [binary, '--profile', profile]

    def call(*args, success=True):
        proc = subprocess.run([*command, *args], capture_output=True, text=True, timeout=30)
        if success:
            assert proc.returncode == 0, (args, proc.stderr)
        else:
            assert proc.returncode == 1 and not proc.stdout and proc.stderr, (args, proc)
        return proc.stdout.strip()

    call('eval', '--focused', '1', success=False)
    assert not profile.exists()
    assert '--view ID' in call('help', 'eval')
    host = subprocess.Popen([*command, '--headless', fixture.as_uri()], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, text=True)
    try:
        assert json.loads(host.stdout.readline())['event'] == 'ready'
        while json.loads(host.stdout.readline()).get('event') != 'loaded':
            pass
        initial = json.loads(call('views'))[0]['view']
        call('eval', '--focused', '1', success=False)
        host.stdin.write(json.dumps({'cmd':'focus','view':initial,'request_id':86})+'\n')
        host.stdin.flush()
        while json.loads(host.stdout.readline()).get('event') != 'focused':
            pass
        assert call('eval','--last-focused','2+2') == '4'
        assert call('eval','--focused','3+4') == '7'
        url = fixture.as_uri()
        # A renderer-created window races two independent open calls.
        point = "var r=document.querySelector('a').getBoundingClientRect(); [Math.round(r.x+r.width/2), Math.round(r.y+r.height/2)]"
        x, y = json.loads(call('eval', '--view', str(initial), point))
        with ThreadPoolExecutor(max_workers=2) as pool:
            opened = [pool.submit(call, 'open', '--wait', url) for _ in range(2)]
            host.stdin.write(json.dumps({'cmd':'click','view':initial,'x':x,'y':y})+'\n')
            host.stdin.flush()
            ids = [future.result() for future in opened]

        assert len(set(ids)) == 2
        for view in ids:
            assert call('eval','--view',view,'document.title') == 'Bug renderer smoke test'
        deadline = time.monotonic() + 10
        while len(json.loads(call('views','--json'))) != 4:
            assert time.monotonic() < deadline, 'link-created view did not open'
            time.sleep(.03)
        assert call('eval','--view',ids[0],'null') == 'null'
        assert json.loads(call('eval','--view',ids[0],'--json','undefined')) == {'type':'undefined'}
        call('eval','--view',ids[0],'throw Error("expected")',success=False)
        call('eval','--view','999999','1',success=False)
        assert call('--', url) == ''
        assert len(json.loads(call('views'))) == 5
        watcher = subprocess.Popen([*command,'watch'],stdout=subprocess.PIPE,text=True)
        try:
            deadline = time.monotonic()+10
            for index in range(100):
                title = f'watch-{index}'
                call('eval','--view',ids[0],f'document.title={json.dumps(title)}')
                if select.select([watcher.stdout],[],[],.1)[0]:
                    event = json.loads(watcher.stdout.readline())
                    if event.get('event')=='title' and event['view']==int(ids[0]):
                        break
                assert time.monotonic() < deadline, 'watch did not report title changes'
            else:
                raise AssertionError('watch did not receive a matching event')
        finally:
            watcher.terminate()
            watcher.wait(timeout=5)
        path = call('socket')
        # A peer that never drains its large replies must not block another client.
        with socket.socket(socket.AF_UNIX) as slow:
            slow.setsockopt(socket.SOL_SOCKET,socket.SO_RCVBUF,4096)
            slow.settimeout(12)
            slow.connect(path)
            payload = (json.dumps({'cmd':'find','name':'x'*32768,'request_id':9})+'\n').encode()
            try:
                for _ in range(1024):
                    slow.sendall(payload)
            except (BrokenPipeError,ConnectionResetError):
                pass
            else:
                raise AssertionError('slow reader was not disconnected')
        assert call('eval','--view',ids[0],'42') == '42'
        # The host's own stdout can also be a stalled pipe. Losing its controller
        # must settle a delegated prompt and leave socket clients usable.
        host.stdin.write(json.dumps({'cmd':'set_request_handlers','view':initial,
                                     'kinds':['confirm'],'request_id':201})+'\n')
        host.stdin.flush()
        while json.loads(host.stdout.readline()).get('reply_to') != 201:
            pass
        host.stdin.write(json.dumps({'cmd':'eval','view':initial,
                                     'script':"document.title='settled '+confirm('slow pipe')"})+'\n')
        host.stdin.flush()
        while json.loads(host.stdout.readline()).get('event') != 'request':
            pass
        def fill_stdout():
            try:
                for _ in range(650):
                    record = memoryview(payload)
                    while record:
                        record = record[os.write(host.stdin.fileno(),record):]
            except (BrokenPipeError,ValueError):
                pass
        writer = threading.Thread(target=fill_stdout,daemon=True)
        writer.start()
        deadline = time.monotonic()+15
        while time.monotonic()<deadline:
            if any(v['view']==initial and v['title']=='settled false' for v in json.loads(call('views'))):
                break
            time.sleep(.05)
        else:
            raise AssertionError('reader stalled a delegated prompt')
        assert call('quit') == ''
        assert host.wait(timeout=15) == 0
        writer.join(timeout=5)
        assert not writer.is_alive()
    finally:
        if host.poll() is None:
            host.terminate()
            try:
                host.wait(timeout=5)
            except subprocess.TimeoutExpired:
                host.kill()
                host.wait()
    call('views',success=False)
print('PASS: profile discovery, popup correlation, focus, JS replies, forwarding, slow-reader disconnect, exit status')
