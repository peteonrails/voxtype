#!/usr/bin/env python3
"""Focused release-transport regressions. Never push or publish anything."""
import hashlib,json,pathlib,re,subprocess,tarfile,tempfile,unittest
R=pathlib.Path(__file__).resolve().parents[1]
class ReleaseTransport(unittest.TestCase):
 def test_docker_build_does_not_reuse_layers(self):
  text=(R/'.github/workflows/build-linux.yml').read_text()
  self.assertIn('docker build --no-cache',text)
 def test_collection_cannot_publish_partial_matrix(self):
  text=(R/'.github/workflows/build-linux.yml').read_text()
  self.assertIn("if: ${{ !cancelled() && needs.build.result == 'success' }}",text)
 def test_rc_tag_must_equal_compiled_version(self):
  text=(R/'.github/workflows/build-linux.yml').read_text()
  self.assertIn('if [[ "${VERSION}" != "${CARGO_VERSION}" ]]; then',text)
 def test_desktop_archive_upload_precedes_checksums(self):
  text=(R/'.github/workflows/build-linux.yml').read_text()
  self.assertIn('scripts/package-desktop-assets.py',text)
  self.assertLess(text.index('scripts/package-desktop-assets.py'),text.index('- name: Generate SHA256SUMS.txt'))
 def test_desktop_archive_contains_exact_assets(self):
  script=R/'scripts/package-desktop-assets.py'
  self.assertTrue(script.is_file(),'Desktop assets release archive builder is missing')
  with tempfile.TemporaryDirectory() as d:
   output=pathlib.Path(d)/'assets.tar.gz'
   x=subprocess.run(['python3',str(script),'--output',str(output)],capture_output=True,text=True)
   self.assertEqual(x.returncode,0,x.stderr)
   expected=['omarchy-plugin/components/SchemaForm.qml','omarchy-plugin/manifest.json','quickshell/voxtype-shared/ShellBorder.qml','examples/osd-packages/aura/voxtype-osd.toml','packaging/scripts/voxtype-configure-launcher']
   with tarfile.open(output) as tf:
    match=re.search(r'^version = "([^"]+)"$',(R/'Cargo.toml').read_text(),re.M)
    assert match is not None
    version=match.group(1)
    names=tf.getnames();prefix=f'voxtype-{version}-desktop-assets/'
    for n in expected:
     self.assertIn(prefix+n,names)
     stream=tf.extractfile(prefix+n);assert stream is not None
     self.assertEqual(stream.read(),(R/n).read_bytes())
    stream=tf.extractfile(prefix+'inventory.json');assert stream is not None
    inventory=json.load(stream)
    self.assertEqual(len(inventory),sum(m.isfile() for m in tf.getmembers())-1)
    for n,sha in inventory.items():
     stream=tf.extractfile(prefix+n);assert stream is not None
     self.assertEqual(hashlib.sha256(stream.read()).hexdigest(),sha)
   first=hashlib.sha256(output.read_bytes()).hexdigest()
   subprocess.run(['python3',str(script),'--output',str(output)],check=True,capture_output=True)
   self.assertEqual(first,hashlib.sha256(output.read_bytes()).hexdigest())
if __name__=='__main__':unittest.main()
