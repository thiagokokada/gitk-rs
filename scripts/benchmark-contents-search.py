#!/usr/bin/env python3
"""Run the ignored release search probe; Linux /proc supplies sampled RSS peaks."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('binary', type=Path)
parser.add_argument('repository', type=Path)
parser.add_argument('output', type=Path)
parser.add_argument('--workers', default='baseline,1,2,4,8')
parser.add_argument('--iterations', type=int, default=5)
parser.add_argument('--commits', type=int, default=100000)
args = parser.parse_args()
workloads = [('rare', 'contents:copy_closure', False),
             ('common', 'contents:version', False),
             ('regex', 'contents:sha256.*=', True),
             ('combined', 'path:pkgs/by-name contents:version', False)]

def memory(pid):
    """Return parent RSS and aggregate live-descendant RSS, in KiB."""
    def rss(process):
        try:
            status = Path(f'/proc/{process}/status').read_text()
            return int(re.search(r'VmRSS:\s+(\d+)', status)[1])
        except (OSError, TypeError):
            return 0
    def children(process):
        found = set()
        try:
            for task in Path(f'/proc/{process}/task').iterdir():
                found.update(map(int, (task / 'children').read_text().split()))
        except OSError:
            pass
        return found
    pending = list(children(pid))
    seen = set()
    total = 0
    while pending:
        child = pending.pop()
        if child in seen:
            continue
        seen.add(child)
        total += rss(child)
        pending.extend(children(child))
    return rss(pid), total

with args.output.open('a') as output, tempfile.TemporaryDirectory() as directory:
    for name, query, regex in workloads:
        for workers in args.workers.split(','):
            for iteration in range(args.iterations + 1):
                env = dict(os.environ, GITK_SEARCH_REPOSITORY=str(args.repository.resolve()),
                           GITK_SEARCH_QUERY=query, GITK_SEARCH_REGEX=str(int(regex)),
                           GITK_SEARCH_COMMITS=str(args.commits),
                           GITK_SEARCH_RESULTS=f'{directory}/results')
                if workers != 'baseline':
                    env['GITK_SEARCH_WORKERS'] = workers
                with open(f'{directory}/log', 'w+') as log:
                    command = [str(args.binary.resolve()), 'profile_metadata_search', '--ignored', '--nocapture']
                    before = resource.getrusage(resource.RUSAGE_CHILDREN)
                    started = time.monotonic()
                    process = subprocess.Popen(command,
                                               env=env, stdout=log, stderr=log)
                    peak_app = peak_children = 0
                    while process.poll() is None:
                        app, children = memory(process.pid)
                        peak_app = max(peak_app, app)
                        peak_children = max(peak_children, children)
                        time.sleep(0.02)
                    log.seek(0)
                    text = log.read()
                    if process.returncode:
                        raise RuntimeError(text)
                    after = resource.getrusage(resource.RUSAGE_CHILDREN)
                    user = after.ru_utime - before.ru_utime
                    system = after.ru_stime - before.ru_stime
                    elapsed = time.monotonic() - started
                result = dict(workload=name, query=query, workers=workers, iteration=iteration,
                              warmup=iteration == 0, app_peak_kib=peak_app,
                              children_peak_kib=peak_children, cpu_percent=100*(user+system)/elapsed,
                              duration_ms=int(re.search(r'complete_ms=(\d+)', text)[1]),
                              first_ms=int(m[1]) if (m := re.search(r'first_ms=(\d+)', text)) else None,
                              matches=int(re.search(r'complete_ms=\d+ loaded=\d+ matches=(\d+)', text)[1]),
                              result_sha256=hashlib.sha256(Path(env['GITK_SEARCH_RESULTS']).read_bytes()).hexdigest())
                output.write(json.dumps(result) + '\n')
                output.flush()
                print(json.dumps(result), flush=True)
