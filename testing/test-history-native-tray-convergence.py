"""Additional final-resume menu convergence on the exact private default journey."""
import hashlib
from pathlib import Path
adapter=Path(__file__).with_name('test-history-native-tray-default.py');data=adapter.read_bytes()
assert hashlib.sha256(data).hexdigest()=='7440df1c3ef4bcb0181695e38e1fa7ed8c80cf0beb140efffdfeb13af99c7fb5'
tail="exec(compile(code,str(base),'exec'),{'__name__':'__main__','__file__':str(base)})"
text=data.decode();assert text.count(tail)==1
extra="""needle=\"  press('Private native history tray','Copy last transcript');\"
assert code.count(needle)==1
code=code.replace(needle,\"  wait(lambda:any(n.name=='Pause History' for n in inside('Private native history tray')),'Final Resume must converge native menu to unpaused state');shot('resumed')\\n\"+needle)
"""
text=text.replace(tail,extra+tail)
print('PRIVATE_FINAL_RESUME_CONVERGENCE',hashlib.sha256(data).hexdigest(),hashlib.sha256(text.encode()).hexdigest(),flush=True)
exec(compile(text,str(adapter),'exec'),{'__name__':'__main__','__file__':str(adapter)})
