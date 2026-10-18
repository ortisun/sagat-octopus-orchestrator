import hashlib,json,os,pathlib,signal,subprocess,sys,tempfile,time
runner=pathlib.Path(sys.argv[1]);source=runner.read_text()
anchor='python3 - "${seconds}" "${started_marker}" "$@" <<\'PY\' &\n'
assert source.count(anchor)==1
body=source.split(anchor,1)[1].split('\nPY\n  active_phase_pid=$!',1)[0]
failures=[]
for sig in [signal.SIGINT,signal.SIGTERM]:
 with tempfile.TemporaryDirectory(prefix='sagat-q5-owned-signal-') as directory:
  root=pathlib.Path(directory);pidfile=root/'child.pid'
  bootstrap='''import os,signal,subprocess
from pathlib import Path
_real_popen=subprocess.Popen
def publication_interrupted(*args,**kwargs):
    actual=_real_popen(*args,**kwargs)
    Path(PIDFILE).write_text(str(actual.pid))
    os.kill(os.getpid(),SIGNAL)
    return actual
subprocess.Popen=publication_interrupted
'''.replace('PIDFILE',repr(str(pidfile))).replace('SIGNAL',str(int(sig)))
  program=root/'wrapper.py';program.write_text(bootstrap+body)
  child_pid=None
  try:
   result=subprocess.run([sys.executable,str(program),'10','-',sys.executable,'-c','import time; time.sleep(60)'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=15)
   child_pid=int(pidfile.read_text())
   try:os.kill(child_pid,0);alive=True
   except ProcessLookupError:alive=False
   print(json.dumps({'signal':sig.name,'wrapper_exit':result.returncode,'child_pid':child_pid,'child_alive_after_wrapper_exit':alive,'runner_sha256':hashlib.sha256(runner.read_bytes()).hexdigest(),'embedded_wrapper_sha256':hashlib.sha256(body.encode()).hexdigest()}),flush=True)
   if result.returncode!=128+sig or alive:failures.append(sig.name)
  finally:
   if child_pid is None and pidfile.exists():child_pid=int(pidfile.read_text())
   if child_pid:
    try:os.killpg(child_pid,signal.SIGKILL)
    except ProcessLookupError:pass
if failures:raise SystemExit('publication signal was not reaped: '+','.join(failures))
