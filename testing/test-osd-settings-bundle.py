#!/usr/bin/env python3
"""Real CLI -> signal -> Panel -> SchemaForm -> picker -> config persistence.

Uses the exact bundle installer helpers and actual setup-produced QML in a
private HOME. The actual form is reparented into a test Window for captures.
No daemon/audio/preview, host configuration, service or shell mutation.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tomllib
from unittest.mock import patch

parser=argparse.ArgumentParser()
parser.add_argument('--bundle',type=Path,required=True)
parser.add_argument('--old-bin',type=Path,required=True)
parser.add_argument('--evidence',type=Path,required=True)
a=parser.parse_args();b=a.bundle.resolve();e=a.evidence.resolve();e.mkdir(parents=True,exist_ok=True)
home=e/'home';home.mkdir(exist_ok=True)
env=dict(os.environ,HOME=str(home),XDG_CONFIG_HOME=str(home/'.config'),XDG_DATA_HOME=str(home/'.local/share'),QT_QUICK_BACKEND='software')
config=home/'.config/voxtype/config.toml';config.parent.mkdir(parents=True,exist_ok=True)
config.write_text('# OSD picker fixture\n[osd]\nstyle = "default" # preserve choice comment\n[whisper]\nlanguage = "de"\n[text]\nfilter_filler_words = false\n')
custom=home/'.local/share/voxtype/osd/aegis-hud';custom.mkdir(parents=True,exist_ok=True)
(custom/'voxtype-osd.toml').write_text('name = "aegis-hud"\ndescription = "Preserved user Aegis"\nlayout = "compact"\n')
(custom/'user-marker').write_text('do not overwrite')

def run(name,args,timeout=35):
 r=subprocess.run([str(x) for x in args],env=env,text=True,capture_output=True,timeout=timeout)
 (e/(name+'.stdout')).write_text(r.stdout);(e/(name+'.stderr')).write_text(r.stderr)
 assert r.returncode==0,(name,r.stdout,r.stderr)
 return r

old=json.loads(run('old-v4-styles',[a.old_bin,'--config',config,'info','styles','--json']).stdout)
before=json.loads(run('candidate-before-styles',[b/'voxtype','--config',config,'info','styles','--json']).stdout)
assert {x['name'] for x in old}=={'default','aegis-hud'},old
assert {x['name'] for x in before}=={'default','aegis-hud'},before
setup=home/'.local/share/voxtype/quickshell'
run('setup-produced',[b/'voxtype','setup','quickshell','--source',b/'quickshell','--target',setup,'--skip-bridge'])
spec=importlib.util.spec_from_file_location('installer',b/'install-local.py');assert spec and spec.loader
m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
state=e/'installer-state';state.mkdir(exist_ok=True)
with patch.dict(os.environ,env):
 staged=m.stage_presets(b,state);meta={'presets_added':[]};m.install_presets(staged,state,meta)
assert (custom/'user-marker').read_text()=='do not overwrite'
styles=json.loads(run('candidate-installed-styles',[b/'voxtype','--config',config,'info','styles','--json']).stdout)
expected=['aura','capsule','halo','shell-dots','shell-line','showcase-bars','showcase-orbit','showcase-prism-scope','showcase-signal-stack']
assert {x['name'] for x in styles}==set(expected+['default','aegis-hud']),styles
root=e/'qs';root.mkdir(exist_ok=True)
for child in Path('/usr/share/omarchy/shell').iterdir():
 if child.is_dir() and child.name!='Ui':(root/child.name).symlink_to(child,target_is_directory=True)
shutil.copytree('/usr/share/omarchy/shell/Ui',root/'Ui')
search=root/'Ui/SearchableDropdown.qml'
search.write_text(search.read_text().replace('id: root','id: root\n readonly property alias listUnderTest: resultList',1))
shutil.copytree(b/'omarchy-plugin',root/'plugin')
panel=root/'plugin/Panel.qml';panel.write_text(panel.read_text().replace('id: root','id: root\n readonly property alias formUnderTest: form\n readonly property alias cliUnderTest: cli',1))
setting=root/'plugin/components/SettingDelegate.qml';setting.write_text(setting.read_text().replace('id: root','id: root\n readonly property alias pickerUnderTest: searchable',1))
wrapper=root/'voxtype-fixture';wrapper.write_text('#!/usr/bin/python3\nimport os,sys\nos.execv('+repr(str((b/'voxtype').resolve()))+',['+repr(str((b/'voxtype').resolve()))+',"--config",'+repr(str(config))+']+sys.argv[1:])\n');wrapper.chmod(0o755)
(root/'plugin/dev.json').write_text(json.dumps({'voxtypeBin':str(wrapper)}))
qml='''import QtQuick
import QtQuick.Window
import Quickshell
import "plugin" as Plugin
Window {
 id: test;visible:true;width:1100;height:950;color:"#202020";title:"VoxType isolated OSD settings test"
 Plugin.Panel { id:panel;manifest:({__sourceDir:__PLUGIN__}) }
 property var expected: __EXPECTED__
 property int selected:0
 property int phase:0
 property int ticks:0
 property var control:null
 property string failure:""
 function check(ok,message){if(!ok&&failure==="")failure=message}
 function styleSpec(){for(var i=0;i<panel.keys.length;i++)if(panel.keys[i].key==="osd.style")return panel.keys[i];return null}
 function finish(){console.log(failure===""?"OSD_PICKER_PASS":"OSD_PICKER_FAIL "+failure);Qt.callLater(Qt.quit)}
 Connections {target:panel.cliUnderTest
  function onWriteFailed(key,message){test.failure=key+":"+message;test.finish()}
  function onSchemaFailed(message){test.failure=message;test.finish()}
 }
 Component.onCompleted:{panel.cliUnderTest.fetchSchema();panel.cliUnderTest.fetchStyles()}
 Timer {interval:180;running:true;repeat:true;onTriggered:{
  ticks++
  if(ticks>130){failure="timeout phase "+phase;finish();return}
  if(phase===0){
   var spec=styleSpec();if(!spec||panel.styles.length!==11)return
   panel.activeSection=String(spec.section)
   panel.formUnderTest.parent=test.contentItem
   panel.formUnderTest.x=0;panel.formUnderTest.y=0
   panel.formUnderTest.width=test.width;panel.formUnderTest.height=test.height
   phase=1;ticks=0
  }else if(phase===1&&ticks>3){
   var form=panel.formUnderTest
   for(var i=0;i<form.visibleKeys.length;i++)if(form.visibleKeys[i].key==="osd.style"){
    form.highlightIndex=i+form.leadingRows;form.scrollTo(form.highlightIndex)
    control=form.rowItem(form.highlightIndex).control
   }
   check(control!==null,"missing rendered style control")
   check(control.controlKind==="searchable","actual searchable picker not used")
   var list=control.optionList;var names=[];var labels=[]
   for(var j=0;j<list.length;j++){names.push(list[j].value);labels.push(list[j].label)}
   for(var k=0;k<expected.length;k++)check(names.indexOf(expected[k])>=0,"Qt sequence dropped "+expected[k])
   check(labels.indexOf("Capsule")>=0&&labels.indexOf("Aura")>=0&&labels.indexOf("Halo")>=0&&labels.indexOf("Shell Line")>=0&&labels.indexOf("Shell Dots")>=0,"human labels missing")
   console.log("ACTUAL_PICKER_OPTIONS "+JSON.stringify(list))
   control.activate();phase=2;ticks=0
  }else if(phase===2&&ticks>4){
   check(control.pickerUnderTest.popupOpen,"picker did not open")
   phase=102
   test.contentItem.grabToImage(function(r){r.saveToFile(__OUT__+"/osd-options-top.png");control.pickerUnderTest.listUnderTest.positionViewAtIndex(5,ListView.Beginning);phase=3;ticks=0})
  }else if(phase===3&&ticks>4){
   phase=103
   test.contentItem.grabToImage(function(r){r.saveToFile(__OUT__+"/osd-options-shell.png");phase=4;ticks=0})
  }else if(phase===4){
   var form=panel.formUnderTest
   control=null
   for(var i=0;i<form.visibleKeys.length;i++)if(form.visibleKeys[i].key==="osd.style")control=form.rowItem(i+form.leadingRows).control
   if(!control)return
   var picker=control.pickerUnderTest;picker.open()
   var index=-1
   for(var j=0;j<picker.filtered.length;j++)if(picker.filtered[j].value===expected[selected])index=j
   check(index>=0,"selection absent "+expected[selected]);picker.listUnderTest.currentIndex=index
   picker.listUnderTest.selectCurrent();phase=5;ticks=0
  }else if(phase===5&&ticks>4){
   if(styleSpec().value!==expected[selected])return
   console.log("PERSISTED_STYLE "+expected[selected])
   selected++;if(selected===expected.length){finish();return}
   phase=4;ticks=0
  }
 }}
}
'''.replace('__PLUGIN__',json.dumps(str(root/'plugin'))).replace('__EXPECTED__',json.dumps(expected)).replace('__OUT__',json.dumps(str(e)))
(root/'shell.qml').write_text(qml)
r=run('qml-picker',['qs','-p',root/'shell.qml'],timeout=40)
assert 'OSD_PICKER_PASS' in r.stdout+r.stderr and 'OSD_PICKER_FAIL' not in r.stdout+r.stderr,(r.stdout,r.stderr)
assert sum(('PERSISTED_STYLE '+name) in r.stdout+r.stderr for name in expected)==len(expected)
saved=tomllib.loads(config.read_text());assert saved['osd']['style']==expected[-1] and saved['whisper']['language']=='de' and not saved['text']['filter_filler_words']
assert '# preserve choice comment' in config.read_text()
# Execute the actual frontend resolver and actual setup-produced QML for every
# selectable new style, without its production overlay, mic or live preview.
render=e/'render';render.mkdir()
(render/'shell.qml').write_text('''import QtQuick
import QtQuick.Window
import Quickshell
import "file:__SHARED__" as VT
Window {id:w;visible:true;width:680;height:240;color:"#202020"
 VT.StyleLoader{id:loader}
 VT.RecipeRenderer{id:renderer;anchors.fill:parent;style:loader.config;daemonState:"recording";peak:0.4;rms:0.2;ring:[0.1,0.3,0.6,0.4,0.2]}
 Timer{interval:450;running:true;onTriggered:{console.log("STYLE_RESOLVED "+JSON.stringify(loader.config));w.contentItem.grabToImage(function(r){r.saveToFile("__OUT__/renderer-"+loader.config.style+".png");Qt.quit()})}}
}
'''.replace('__SHARED__',str(setup/'voxtype-shared')).replace('__OUT__',str(e)))
resolved=[]
for name in expected:
 r=run('renderer-'+name,[b/'voxtype-osd-quickshell','--no-daemonize','--config',config,'--style',name,'--qml-path',render])
 logs=r.stdout+r.stderr;assert 'STYLE_RESOLVED ' in logs,logs
 line=next(x.split('STYLE_RESOLVED ',1)[1] for x in logs.splitlines() if 'STYLE_RESOLVED ' in x);data=json.loads(line)
 assert data['style']==name and data['package_dir']==str(home/'.local/share/voxtype/osd'/name),(name,data)
 manifest=tomllib.loads((home/'.local/share/voxtype/osd'/name/'voxtype-osd.toml').read_text())
 assert data['layout']==manifest['layout'] and data['frame']['border']==manifest['frame']['border'],(name,data)
 assert not any(x in logs for x in ['ReferenceError:','TypeError:','Failed to load configuration'])
 resolved.append({'name':name,'layout':data['layout'],'package_dir':data['package_dir']})
(e/'persisted-config.toml').write_bytes(config.read_bytes())
(e/'result.json').write_text(json.dumps({'passed':True,'styles':styles,'old_v4_before_install':old,'candidate_before_install':before,'resolved':resolved,'application_sha256':hashlib.sha256((b/'voxtype').read_bytes()).hexdigest(),'plugin_panel_sha256':hashlib.sha256((b/'omarchy-plugin/Panel.qml').read_bytes()).hexdigest(),'custom_aegis_preserved':True,'scope':'actual bundle installer preset helpers; real setup CLI; real Panel/SchemaForm/SearchableDropdown persistence; actual frontend resolver and setup-produced renderer in isolated Window, no live preview/production changes'},indent=2)+'\n')
print('OSD_SETTINGS_INTEGRATION_PASS')
