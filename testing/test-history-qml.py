"""Real Qt history component + real CLI, private NAS fixtures only."""
import datetime
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

from PySide6.QtCore import QObject, QUrl, Signal, Slot, Qt, QPointF
from PySide6.QtGui import QGuiApplication
from PySide6.QtQml import QQmlComponent, QQmlEngine
from PySide6.QtQuick import QQuickWindow
from PySide6.QtTest import QTest

assert os.environ.get('HOME') == '/work/home', 'NAS fixture only'
os.environ['QT_QPA_PLATFORM'] = 'offscreen'
os.environ['QT_QUICK_BACKEND'] = 'software'
os.umask(0o077)
exe = Path(os.environ.get('VOXTYPE_HISTORY_TEST_BIN','/work/target/debug/voxtype'))
source = Path(__file__).resolve().parents[1]
app = QGuiApplication([])
engine = QQmlEngine()

with tempfile.TemporaryDirectory(dir='/work/scratch', prefix='history-qml-') as name:
    root = Path(name)
    env = dict(os.environ, HOME=str(root), XDG_CONFIG_HOME=str(root/'config'), XDG_DATA_HOME=str(root/'data'), XDG_RUNTIME_DIR=str(root/'runtime'))
    (root/'runtime').mkdir(mode=0o700)
    config = root/'config.toml'
    config.write_text('[hotkey]\nenabled=false\n[history]\nenabled=true\n')
    sink=root/'clipboard-capture'
    if os.environ.get('VOX_HISTORY_QML_CASE') in ('copy','paste'):
        tools=root/'tools'
        tools.mkdir(mode=0o700)

        helper=tools/'wl-copy'
        helper.write_text('#!/usr/bin/python3\nimport os,sys\nfrom pathlib import Path\nPath(os.environ["VOX_HISTORY_CAPTURE"]).write_bytes(sys.stdin.buffer.read())\n')
        helper.chmod(0o700)
        env.update(PATH=str(tools)+':'+env['PATH'],WAYLAND_DISPLAY='history-fixture-no-compositor',XDG_SESSION_TYPE='wayland',VOX_HISTORY_CAPTURE=str(sink))
        if os.environ.get('VOX_HISTORY_QML_CASE') == 'paste':
            config.write_text(config.read_text()+'[output]\nmode="clipboard"\nappend_text=""\nwait_for_modifier_release=false\n')
    day = datetime.datetime.now().astimezone()
    entry_id = day.strftime('%Y%m%d-%H%M%S')+'-abcdef'
    folder = root/'data/voxtype/history'
    folder.mkdir(parents=True, mode=0o700)
    entry = dict(schema=1,id=entry_id,ts_start=day.isoformat(),ts_end=day.isoformat(),raw_text='um synthetic invoice',text='Synthetic invoice.',engine='whisper',model='synthetic-model.gguf',backend='cpu',variant='test',audio_ms=1200,transcribe_ms=20,streaming=False,voxtype_version='fixture',delivered=False,outcome='output_failed',trigger='record',ended_by='released',auto_submit=False,app='synthetic-editor',edits=[{'from':'um','to':''}])
    if os.environ.get('VOX_HISTORY_QML_CASE') == 'raw-fallback':
        entry.update(text='',edits=[])
    (folder/(day.strftime('%Y-%m-%d')+'.jsonl')).write_text(json.dumps(entry)+'\n')

    class Client(QObject):
        historyReply = Signal(str, bool, 'QVariant', str)
        def __init__(self):
            super().__init__()
            self.commands = []
        @Slot(str, 'QVariantList')
        def historyRequest(self, token, args):
            self.commands.append(list(args))
            result = subprocess.run([str(exe),'--config',str(config),'history','--json',*args],env=env,capture_output=True,text=True,timeout=20)
            try:
                value = json.loads(result.stdout)
            except ValueError:
                value = None
            self.historyReply.emit(token,result.returncode==0,value,result.stderr)

    client = Client()
    surface=os.environ.get('VOX_HISTORY_QML_SURFACE','plugin')
    assert surface in ['plugin','standalone']
    component_path=source/('omarchy-plugin/components/HistoryPanel.qml' if surface=='plugin' else 'quickshell/voxtype-meetings/HistoryPanel.qml')
    component = QQmlComponent(engine,QUrl.fromLocalFile(str(component_path)))
    assert component.status() == QQmlComponent.Ready, '\n'.join(e.toString() for e in component.errors())
    panel = component.createWithInitialProperties({'client':client})
    assert panel is not None, '\n'.join(e.toString() for e in component.errors())
    window = QQuickWindow()
    window.setColor('#111318')
    window.resize(1100,760)
    panel.setParentItem(window.contentItem())
    panel.setWidth(1100)
    panel.setHeight(760)
    window.show()
    def wait(check):
        deadline = time.monotonic()+8
        while not check():
            assert time.monotonic()<deadline, 'Qt history assertion timed out'
            app.processEvents()
            time.sleep(0.02)
    wait(lambda: panel.property('entryCount') == 1)
    assert client.commands[0] == ['list']
    assert panel.property('stateText') == 'History on'
    def visual_items(item):
        yield item
        for child in item.childItems():
            yield from visual_items(child)
    # Delegates are visual children, not necessarily QObject-owned children.
    labels = lambda: '\n'.join(str(o.property('text') or '') for o in visual_items(window.contentItem()))
    def click_named(name):
        item = next((o for o in visual_items(window.contentItem()) if o.objectName()==name),None)
        assert item is not None, 'Missing history control: '+name
        wait(lambda: item.isVisible() and item.width()>0 and item.height()>0)
        # Rows re-lay out when they enter and leave correction; a position
        # read before the next frame is the old one.
        QTest.qWait(60)
        QTest.mouseClick(window,Qt.LeftButton,Qt.NoModifier,item.mapToScene(QPointF(item.width()/2,item.height()/2)).toPoint())
        app.processEvents()
    wait(lambda: 'synthetic-editor' in labels() and 'synthetic-model.gguf' in labels())
    assert '[failed]' in labels()
    wait(lambda: not window.grabWindow().isNull())
    capture = Path('/work/out/history-qml-list.png')
    assert window.grabWindow().save(str(capture))
    if os.environ.get('VOX_HISTORY_QML_CASE') == 'raw-fallback':
        def control(name):
            return next(o for o in visual_items(window.contentItem()) if o.objectName()==name)
        assert control('history-copy-'+entry_id).isEnabled(), 'Raw-only dictation must retain its copy action'
        assert control('history-paste-'+entry_id).isEnabled(), 'Raw-only dictation must retain its replay action'
        assert any(o.isVisible() and o.property('text')==entry['raw_text'] for o in visual_items(window.contentItem())), 'Raw-only row must show original engine wording'
        click_named('history-correct-'+entry_id)
        assert control('history-correction-text').property('text')==entry['raw_text'], 'Raw-only correction editor must start with engine wording'
        click_named('history-correction-cancel')
        result=subprocess.run([str(exe),'--config',str(config),'history','--json','correct',entry_id,'--',''],env=env,capture_output=True,text=True,timeout=15)
        assert result.returncode==0,result.stderr
        panel.refresh()
        wait(lambda: '[corrected]' in labels())
        assert not control('history-copy-'+entry_id).isEnabled(), 'Intentional empty correction must not recover raw text'
        assert not control('history-paste-'+entry_id).isEnabled()
        click_named('history-correct-'+entry_id)
        assert control('history-correction-text').property('text')==''
        click_named('history-correction-cancel')
        raw_capture=Path('/work/out/history-qml-raw-fallback-'+surface+'.png')
        assert not raw_capture.exists(), 'Each raw-fallback surface needs a fresh capture'
        assert window.grabWindow().save(str(raw_capture))
    if os.environ.get('VOX_HISTORY_QML_CASE') == 'controls':
        click_named('history-pause')
        wait(lambda: panel.property('stateText')=='History paused')
        assert ['pause'] in client.commands
        click_named('history-pause')
        wait(lambda: panel.property('stateText')=='History on')
        assert ['resume'] in client.commands
        click_named('history-enabled')
        wait(lambda: panel.property('stateText')=='History off')
        click_named('history-enabled')
        wait(lambda: panel.property('stateText')=='History on')
        assert ['disable'] in client.commands and ['enable'] in client.commands
        assert panel.property('entryCount')==1, 'Toggling collection must not delete existing history'
        assert window.grabWindow().save('/work/out/history-qml-controls.png')
    if os.environ.get('VOX_HISTORY_QML_CASE') == 'paste':
        click_named('history-paste-'+entry_id)
        QTest.qWait(200)
        assert window.grabWindow().save('/work/out/history-qml-paste.png')
        hidden=[]
        panel.pasteRequested.connect(lambda: (hidden.append(True),window.hide()))
        click_named('history-paste-confirm')
        wait(lambda: sink.exists())
        assert sink.read_text()=='Synthetic invoice.'
        assert hidden and not window.isVisible(), 'Replay must release the history window before output'
        assert any(args[:2]==['paste',entry_id] for args in client.commands)
    if os.environ.get('VOX_HISTORY_QML_CASE') == 'copy':
        click_named('history-copy-'+entry_id)
        wait(lambda: sink.exists())
        assert sink.read_text()=='Synthetic invoice.'
        assert ['copy',entry_id] in client.commands
        assert panel.property('entryCount')==1
        assert window.grabWindow().save('/work/out/history-qml-copy.png')
    if os.environ.get('VOX_HISTORY_QML_CASE') == 'delete':
        click_named('history-delete-'+entry_id)
        click_named('history-delete-cancel')
        listed=subprocess.run([str(exe),'--config',str(config),'history','--json','list'],env=env,capture_output=True,text=True,timeout=15)
        assert listed.returncode==0 and len(json.loads(listed.stdout)['entries'])==1
        click_named('history-delete-'+entry_id)
        click_named('history-delete-confirm')
        wait(lambda: panel.property('entryCount')==0)
        listed=subprocess.run([str(exe),'--config',str(config),'history','--json','list'],env=env,capture_output=True,text=True,timeout=15)
        assert listed.returncode==0 and json.loads(listed.stdout)['entries']==[]
        assert 'No dictations stored' in labels()
        assert window.grabWindow().save('/work/out/history-qml-delete.png')
    if os.environ.get('VOX_HISTORY_QML_CASE') == 'correction':
        def current_entry():
            result=subprocess.run([str(exe),'--config',str(config),'history','--json','show',entry_id],env=env,capture_output=True,text=True,timeout=15)
            assert result.returncode==0,result.stderr
            return json.loads(result.stdout)
        click_named('history-correct-'+entry_id)
        editor=next(o for o in visual_items(window.contentItem()) if o.objectName()=='history-correction-text')
        editor.setProperty('text','Cancelled correction')
        click_named('history-correction-cancel')
        assert current_entry().get('corrected_text') is None
        click_named('history-correct-'+entry_id)
        editor.setProperty('text','- Corrected invoice.\nSecond line.')
        click_named('history-correction-save')
        wait(lambda:'[corrected]' in labels())
        saved=current_entry()
        assert saved['corrected_text']=='- Corrected invoice.\nSecond line.'
        assert saved['raw_text']=='um synthetic invoice' and saved['text']=='Synthetic invoice.'
        click_named('history-correct-'+entry_id)
        editor.setProperty('text','')
        click_named('history-correction-save')
        assert current_entry()['corrected_text']=='', 'Empty correction must not fall back to the original'
        assert window.grabWindow().save('/work/out/history-qml-correction.png')
    if os.environ.get('VOX_HISTORY_QML_CASE') == 'details':
        click_named('history-details-'+entry_id)
        wait(lambda: 'Raw:' in labels() and 'um synthetic invoice' in labels())
        assert 'Automatic edits:' in labels()
        assert '- um' in labels()
        assert window.grabWindow().save('/work/out/history-qml-details.png')
    if os.environ.get('VOX_HISTORY_QML_CASE') == 'search':
        search = next((o for o in visual_items(panel) if o.objectName() == 'history-search'), None)
        assert search is not None, 'History needs its unified search field'
        # Search runs as you type: there is no submit button.
        assert not any(o.objectName() == 'history-search-submit' for o in visual_items(panel))
        def type_query(text):
            click_named('history-search')
            QTest.keySequence(window, 'Ctrl+A')
            for ch in text:
                QTest.keyClick(window, ch)
        type_query('invoice')
        wait(lambda: ['search','invoice'] in client.commands)
        wait(lambda: panel.property('entryCount') == 1)
        type_query('definitely-not-present')
        wait(lambda: panel.property('entryCount') == 0)
        assert 'No matches' in labels()
        assert window.grabWindow().save('/work/out/history-qml-search.png')
    print(json.dumps({'case':'qml-real-cli-'+os.environ.get('VOX_HISTORY_QML_CASE','list'),'entries':panel.property('entryCount'),'commands':client.commands,'capture':str(capture),'boundary':'private synthetic fixture, offscreen Qt; not Quickshell host or physical desktop acceptance'}))
    window.close()
    panel.deleteLater()
    app.processEvents()
