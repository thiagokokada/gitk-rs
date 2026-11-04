#!/usr/bin/env python3
"""Small X11 release observation, not a statistical benchmark.

Usage: release_probe.py BINARY go|rust REPOSITORY [OUTPUT_DIRECTORY]
Requires git, xdotool, xwininfo, ImageMagick import and X11/XRes libraries.
Only windows whose XRes client PID matches the child process are touched.
"""
import json, os, re, signal, subprocess, sys, tempfile, time
import shutil
from pathlib import Path

binary, label, repository = sys.argv[1:4]
assert label in ['go', 'rust'], 'app must be go or rust'
root = Path(sys.argv[4] if len(sys.argv) > 4 else '/tmp/gitk-rs-release-observations')
root.mkdir(parents=True, exist_ok=True)
cache = Path(tempfile.mkdtemp(prefix=label+'-cache-', dir=root))
config = Path(tempfile.mkdtemp(prefix=label+'-config-', dir=root))
env = os.environ.copy()
env.pop('WAYLAND_DISPLAY', None)
env['XDG_CACHE_HOME'] = str(cache)
env['XDG_CONFIG_HOME'] = str(config)
args = [binary, '--limit', '192', '--mode', 'dark', '--nowatch', '--verbose']
if label == 'rust': args += ['--timings']
args += [repository]
records = []
import ctypes as C
res_path = os.environ.get('GITK_RS_XRES_LIBRARY', 'libXRes.so.1')
x = C.CDLL('libX11.so.6'); res = C.CDLL(res_path)
x.XOpenDisplay.restype = C.c_void_p
display = x.XOpenDisplay(None)
if not display: raise RuntimeError("An X11 display is required")
class Spec(C.Structure): _fields_ = [('client', C.c_ulong), ('mask', C.c_uint)]
class Value(C.Structure): _fields_ = [('spec', Spec), ('length', C.c_long), ('value', C.c_void_p)]
res.XResQueryClientIds.argtypes = [C.c_void_p, C.c_long, C.POINTER(Spec), C.POINTER(C.c_long), C.POINTER(C.POINTER(Value))]
res.XResGetClientPid.argtypes = [C.POINTER(Value)]; res.XResGetClientPid.restype = C.c_int
res.XResClientIdsDestroy.argtypes = [C.c_long, C.POINTER(Value)]
def owns_window(window, pid):
    spec = Spec(int(window), 2); count = C.c_long(); values = C.POINTER(Value)()
    res.XResQueryClientIds(display, 1, C.byref(spec), C.byref(count), C.byref(values))
    try: return any(res.XResGetClientPid(C.byref(values[i])) == pid for i in range(count.value))
    finally:
        if values: res.XResClientIdsDestroy(count, values)


def memory(pid):
    try:
        data = Path(f'/proc/{pid}/status').read_text()
        return {key: int(re.search(r'^'+key+r':\s+(\d+)', data, re.M)[1]) for key in ['VmRSS','VmHWM']}
    except (OSError, TypeError): return {'VmRSS':0, 'VmHWM':0}

def children(pid):
    direct = set()
    try:
        for thread in Path(f'/proc/{pid}/task').iterdir():
            try: direct.update(int(value) for value in (thread/'children').read_text().split())
            except OSError: pass
    except OSError: pass
    return list(direct) + [descendant for child in direct for descendant in children(child)]

for trial in ['first-launch', 'warm-reopen']:
    logfile = root / f'{label}-{trial}.log'
    started = time.monotonic()
    with logfile.open('w') as output:
        app = subprocess.Popen(args, env=env, stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
        child_peak = 0
        app_peak = 0
        first_history = None
        observed = {}
        window = None
        try:
            while time.monotonic() - started < 8:
                if app.poll() is not None: raise RuntimeError(f'{label} exited {app.returncode}: {logfile.read_text()[-2000:]}')
                if window is None and time.monotonic() - started > 0.5:
                    tool = shutil.which('xdotool')
                    assert tool, 'xdotool is missing'
                    found = subprocess.run([tool, 'search', '--all', '--onlyvisible', '--name', '^'+('gitk-rs' if label == 'rust' else 'gitk-go')+'$'], env=env, capture_output=True, text=True)
                    if found.returncode == 0:
                        window = next((value for value in found.stdout.splitlines() if owns_window(value, app.pid)), None)
                        if window is None: continue
                        subprocess.run([tool, 'windowsize', '--sync', window, '1920', '1332'], env=env, timeout=5, check=True)
                observed = memory(app.pid)
                app_peak = max(app_peak, observed['VmHWM'])
                child_peak = max(child_peak, sum(memory(child)['VmRSS'] for child in children(app.pid)))
                log = logfile.read_text()
                marker = 'phase=HistoryPainted' if label == 'rust' else 'reloadCommitsAsync loaded'
                if first_history is None and marker in log: first_history = round((time.monotonic() - started)*1000, 1)
                time.sleep(0.025)
            assert window is not None, 'own window not found'
            window_info = subprocess.check_output(['xwininfo','-id', window], text=True, env=env)
            window = re.search(r'Window id: (0x[0-9a-f]+)', window_info)[1]
            subprocess.run(['import','-window',window,str(root/f'{label}-{trial}.png')], env=env, check=True)
            record = {'app':label, 'trial':trial, 'duration_s':8, 'width':int(re.search(r'Width: (\d+)', window_info)[1]), 'height':int(re.search(r'Height: (\d+)', window_info)[1]), 'rss_kib_at_end':observed['VmRSS'], 'app_vm_hwm_kib':app_peak,
                'sampled_children_peak_total_rss_kib':child_peak, 'first_history_log_observed_ms':first_history,
                'first_history_event':'CPU paint callback' if label == 'rust' else 'Go controller batch applied',
                'log':str(logfile)}
            records.append(record)
            print(json.dumps(record), flush=True)
        finally:
            if app.poll() is None:
                os.killpg(app.pid, signal.SIGTERM)
                try: app.wait(timeout=5)
                except subprocess.TimeoutExpired: os.killpg(app.pid, signal.SIGKILL); app.wait()
(root/f'{label}.json').write_text(json.dumps(records, indent=2)+'\n')
