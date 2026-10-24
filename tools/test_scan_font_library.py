import sys,tempfile,unittest,json
from pathlib import Path
sys.path.insert(0,'tools')
import scan_font_library as scan
class ScannerTests(unittest.TestCase):
 def test_document_discovery_and_exclusion(self):
  with tempfile.TemporaryDirectory() as tmp:
   root=Path(tmp);(root/'font.ttf').write_bytes(b'font');(root/'data.json').write_text('{}');(root/'font.json').write_text('{"format":"typefoundry.font"}');(root/'do-not-index').mkdir();(root/'do-not-index'/'private.ttf').write_bytes(b'font')
   paths,errors=scan.discover([root]);self.assertEqual(len(paths),2);self.assertEqual(errors,[])
 def test_hash_and_atomic_metadata_only(self):
  with tempfile.TemporaryDirectory() as tmp:
   root=Path(tmp);p=root/'font.ttf';p.write_bytes(b'font');self.assertEqual(scan.digest(p),scan.hashlib.sha256(b'font').hexdigest());scan.atomic_json(root/'out.json',{'a':1});scan.atomic_json(root/'out.json',{'a':2});self.assertEqual(json.loads((root/'out.json').read_text()),{'a':2})
 def test_migration_reentrant(self):
  db=scan.sqlite3.connect(':memory:');sql=Path('tools/migrations/001_font_library.sql').read_text(encoding='utf-8-sig');db.executescript(sql);db.executescript(sql);self.assertEqual(db.execute('pragma user_version').fetchone()[0],1);db.close()
class EndToEndTests(unittest.TestCase):
 def test_engine_scan_resume_and_content_aliases(self):
  from unittest.mock import patch
  with tempfile.TemporaryDirectory() as tmp:
   root=Path(tmp);inputs=root/'inputs';inputs.mkdir();fixture=Path('App/crates/foundry-core/tests/fixtures/OriginalVariable.ttf').read_bytes();(inputs/'a.ttf').write_bytes(fixture);(inputs/'b.ttf').write_bytes(fixture)
   store=root/'store';exe=Path('App/target/release/foundry.exe').resolve()
   if not exe.exists():self.skipTest('release CLI needed for orchestration integration test')
   argv=['scan','--foundry',str(exe),'--store',str(store),'--roots',str(inputs),'--jobs','2']
   for _ in range(2):
    with patch.object(sys,'argv',argv):self.assertEqual(scan.main(),0)
   progress=json.loads((store/'progress.json').read_text());self.assertEqual(progress['counts']['measured'],2);self.assertEqual(progress['unique_contents'],1)
   objects=list((store/'studies').glob('*.json'));self.assertEqual(len(objects),1);self.assertEqual(len(json.loads(objects[0].read_text())['fonts']),3)
class MigrationSafetyTests(unittest.TestCase):
 def test_future_database_version_is_not_downgraded(self):
  from unittest.mock import patch
  with tempfile.TemporaryDirectory() as tmp:
   root=Path(tmp);db=scan.sqlite3.connect(root/'scan.sqlite');db.execute('pragma user_version=99');db.close()
   exe=Path('App/target/release/foundry.exe').resolve()
   if not exe.exists():self.skipTest('release CLI needed')
   with patch.object(sys,'argv',['scan','--foundry',str(exe),'--store',str(root)]):
    with self.assertRaisesRegex(RuntimeError,'unsupported worker database'):scan.main()
   db=scan.sqlite3.connect(root/'scan.sqlite');self.assertEqual(db.execute('pragma user_version').fetchone()[0],99);db.close()
unittest.main()
