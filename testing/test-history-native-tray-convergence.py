"""Additional final-resume menu convergence on the exact private default journey."""
import hashlib
from pathlib import Path
adapter=Path(__file__).with_name('test-history-native-tray-default.py');data=adapter.read_bytes()
assert hashlib.sha256(data).hexdigest()=='58611c171070489ba2c80617d6ad98d50e1d16211797cb5fd3c2ce2b3a36c66b'
tail="exec(compile(code,str(base),'exec'),{'__name__':'__main__','__file__':str(base)})"
text=data.decode();assert text.count(tail)==1
extra="""needle=\"  press('Private native history tray','Copy last transcript');\"
assert code.count(needle)==1
code=code.replace(needle,\"  wait(lambda:any(n.name=='Pause History' for n in inside('Private native history tray')),'Final Resume must converge native menu to unpaused state');shot('resumed')\\n\"+needle)
"""
text=text.replace(tail,extra+tail)
print('PRIVATE_FINAL_RESUME_CONVERGENCE',hashlib.sha256(data).hexdigest(),hashlib.sha256(text.encode()).hexdigest(),flush=True)
exec(compile(text,str(adapter),'exec'),{'__name__':'__main__','__file__':str(adapter)})
