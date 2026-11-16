"""Real EXTERNAL edits through selected-file watcher, not write callbacks.
Reuses the canonical staged Panel fixture, with readonly schema observations.
"""
import hashlib,os
from pathlib import Path
base=Path(__file__).with_name('test-history-settings-graphical.py');raw=base.read_bytes()
assert hashlib.sha256(raw).hexdigest()=='e240164315ac634af5ba17be404b473af9435b4f190acc78c7691303f2646d12'
kind=os.environ['VOX_HISTORY_SETTINGS_EXTERNAL_CASE'];assert kind in ['profile','history','pending-history','replacements','scalar']
code=raw.decode()
needle=" assert chosen.stat().st_mode&0o777==0o600 and not Path(str(chosen)+'.tmp').exists()"
assert code.count(needle)==1
code=code.replace(needle," chosen.write_text(chosen.read_text()+'\\n[profiles.fixture.vocabulary]\\nterms = [\"private-old\"]\\n')\n"+needle)
needle=' property int serial:0';assert code.count(needle)==1
code=code.replace(needle,' property int schemaRevision:0\n Connections { target:panel.item?panel.item.cliTest:null; function onSchemaLoaded(next){fixture.schemaRevision++} }\n'+needle)
needle='restartNeeded:p?p.restartNeeded:false,';assert code.count(needle)==1
code=code.replace(needle,needle+'schemaRevision:fixture.schemaRevision,changedAt:p?p.configChangedAt:0,profiles:p&&p.schema?p.schema.profiles:null,replacements:p&&p.schema?p.schema.replacements:null,restartBannerRendered:p?fixture.find(p.renderTest,function(x){return x.text!==undefined&&String(x.text).indexOf("Changes take effect after restart")>=0&&fixture.shown(x)})!==null:false,')
start=code.index('  initial=resolved();assert ');end=code.index('  assert not select.select([audio]',start)
journey=r'''  external_case=os.environ['VOX_HISTORY_SETTINGS_EXTERNAL_CASE']
  wait(lambda:not status().get('schemaLoading') and status().get('profiles',{}).get('fixture',{}).get('vocabulary',{}).get('terms')==['private-old'],'Initial complete profile schema')
  assert not status()['restartNeeded'] and not status()['restartBannerRendered']
  shot('initial')
  def edit_selected(label,edit,observed,expected_restart):
   before=status();before_text=chosen.read_text();after=edit(before_text)
   assert after!=before_text
   before_data=tomllib.loads(before_text);after_data=tomllib.loads(after)
   if label=='profile':
    import copy
    expected=copy.deepcopy(before_data);expected['profiles']['fixture']['vocabulary']['terms']=['private-new']
    assert after_data==expected,'Profile RED must edit ONLY vocabulary terms'
   chosen.write_text(after)
   assert chosen.stat().st_mode&0o777==0o600 and sentinel.read_bytes()==untouched
   def refreshed():
    now=status()
    return now.get('changedAt',0)>before.get('changedAt',0) and now.get('schemaRevision',0)>before.get('schemaRevision',0) and not now.get('schemaLoading') and observed(now)
   wait(refreshed,'Actual selected file-watch and COMPLETED canonical schema refresh for '+label)
   state=status();steps.append(dict(external_case=external_case,label=label,before_schema_revision=before['schemaRevision'],after_schema_revision=state['schemaRevision'],watcher_changed=True,restart=state['restartNeeded'],rendered_banner=state['restartBannerRendered'],selected_only=True))
   assert state['restartNeeded']==expected_restart,('External-edit restart classification wrong AFTER completed real schema reload',label,state)
   wait(lambda:status()['restartBannerRendered']==expected_restart,'Rendered restart banner must match canonical external edit '+label)
  profile_edit=lambda text:text.replace('terms = ["private-old"]','terms = ["private-new"]',1)
  profile_seen=lambda st:st.get('profiles',{}).get('fixture',{}).get('vocabulary',{}).get('terms')==['private-new']
  history_edit=lambda text:text+'\n[history]\nretention_days=7\n'
  history_seen=lambda st:st.get('values',{}).get('history.retention_days')==7
  if external_case in ['profile','pending-history']:
   edit_selected('profile',profile_edit,profile_seen,True)
   if external_case=='pending-history':edit_selected('history-after-profile',history_edit,history_seen,True)
  elif external_case=='history':edit_selected('history-only',history_edit,history_seen,False)
  elif external_case=='replacements':edit_selected('replacements',lambda text:text+'\n[text.replacements]\nprivate_old="private_new"\n',lambda st:st.get('replacements',{}).get('private_old')=='private_new',True)
  elif external_case=='scalar':
   assert ipc('io.voxtype.settings','section','Text Cleanup')=='ok'
   wait(lambda:status().get('section')=='Text Cleanup' and 'text.filter_filler_words' in status().get('values',{}),'Existing scalar rendered control')
   edit_selected('scalar',lambda text:text.replace('filter_filler_words=false','filter_filler_words=true',1),lambda st:st.get('values',{}).get('text.filter_filler_words') is True,True)
  assert sentinel.read_bytes()==untouched
  assert '# preserve model' in chosen.read_text() and '# preserve unrelated' in chosen.read_text()
  shot('saved');shot('reopened')
'''
code=code[:start]+journey+code[end:]
code=code.replace("history-settings-'+case", "history-settings-'+case+'-'+os.environ['VOX_HISTORY_SETTINGS_EXTERNAL_CASE']")
print('ACTUAL_EXTERNAL_WATCH_FIXTURE',kind,hashlib.sha256(raw).hexdigest(),hashlib.sha256(code.encode()).hexdigest(),flush=True)
exec(compile(code,str(base),'exec'),{'__name__':'__main__','__file__':str(base)})
