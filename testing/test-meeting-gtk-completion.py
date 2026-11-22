"""Adapt only frozen222's fixture callbacks, never product/native source.
The bootstrap case establishes its existing private recording feed. The distinct
completion case controls our new witness; all real CLI arguments remain native.
"""
import hashlib,os
from pathlib import Path
base=Path(__file__).with_name('test-meeting-gtk-live-forwarding.py')
data=base.read_bytes();assert hashlib.sha256(data).hexdigest()=='6c8d2b0d58884ef933287012de22786de564c400bd559d944e486be999e821f0'
kind,transition=os.environ['VOX_MEETING_COMPLETION_CASE'].split(':')
assert kind in ['rename','notes'] and transition in ['current','same-reconnect','manual','live','reconnect','context','return']
text=data.decode();needle='from meeting_view_lifetime import';assert text.count(needle)==3
text=text.replace(needle,'from meeting_completion_lifetime import')
os.environ['VOX_MEETING_LIVE_CASE']='lifetime-rename-same'
print('PRIVATE_COMPLETION_FIXTURE_ADAPTER',os.environ['VOX_MEETING_COMPLETION_CASE'],hashlib.sha256(data).hexdigest(),hashlib.sha256(text.encode()).hexdigest(),'fixture imports only; no product/native modifications',flush=True)
exec(compile(text,str(base),'exec'),{'__name__':'__main__','__file__':str(base)})
