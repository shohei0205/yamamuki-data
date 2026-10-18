"""取得なしの国土地理院 JSON の配布と、公開前の添付ファイル検証を確かめる。"""

import gzip
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from release_tools import gsi_peaks as target


from document_metadata import metadata_maps

class GsiReleaseTests(unittest.TestCase):
    def test_supplement_sources_survive_prepare_and_snapshot(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            _, manifest = target.prepare('test-supplements', root / 'release', 'dev')
            expected = metadata_maps(json.loads((target.DATA / 'supplements.json').read_bytes())['metadata'])['sources']
            self.assertEqual({k:v for k,v in metadata_maps(manifest['metadata'])["sources"].items() if not k.startswith('gsi-')}, expected)
            self.assertEqual(metadata_maps(json.loads((root / 'release' / target.FILE).read_bytes())['metadata'])['tags']['日本百名山']['nameReading'],'にほんひゃくめいざん')
            manifest['sourceFiles'] = [{'url': 'https://example.com/mountains.csv',
                                    'fetchedAt': '2026-10-14T00:00:00Z'}]
            (root / 'release/manifest.json').write_text(json.dumps(manifest), encoding='utf-8')
            info = target.snapshot(root / 'release', root / 'staged')
            self.assertFalse(set(expected) & metadata_maps({'sources':info['sources']})['sources'].keys())
            self.assertEqual(manifest['metadata']['tags'], json.loads((root/'release'/target.FILE).read_bytes())['metadata']['tags'])
            self.assertNotIn('tags', info)

    def test_png_dem_sources_keep_layer_and_template(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root / 'generated'
            _, manifest = target.prepare('test-dem', source, 'dev')
            manifest['sourceFiles'] = [
                {'url': f'https://cyberjapandata.gsi.go.jp/xyz/{layer}/{zoom}/100/200.png',
                 'fetchedAt': '2026-10-14T00:00:00Z'}
                for layer, zoom in [('dem1a_png', 17), ('dem5a_png', 15), ('dem_png', 14)]]
            (source / 'manifest.json').write_text(json.dumps(manifest), encoding='utf-8')
            info = target.snapshot(source, root / 'staged')
            self.assertEqual(set(info['sourceSnapshot']), {'dem1a_png', 'dem5a_png', 'dem_png'})
            self.assertEqual(info['sourceSnapshot']['dem1a_png']['urlTemplate'],
                             'https://cyberjapandata.gsi.go.jp/xyz/dem1a_png/17/{x}/{y}.png')

    def test_vector_id_and_source_are_preserved_in_release_snapshot(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source, staged = root / 'generated', root / 'staged'
            _, manifest = target.prepare('test-vector', source, 'dev')
            points = json.loads((source / target.FILE).read_bytes())
            previous_id = points['points'][0]['id']
            points['points'][0]['id'] = 'gsi-vector:test'
            original_path = source / 'gsi-peaks-source.json.gz'
            original = json.loads(gzip.decompress(original_path.read_bytes()))
            for point in original['points']:
                if point['id'] == previous_id:point['id'] = 'gsi-vector:test'
            original_path.write_bytes(gzip.compress(json.dumps(original).encode(),mtime=0))
            supplement_path = source / 'supplements.json.gz'
            supplement = json.loads(gzip.decompress(supplement_path.read_bytes()))
            for point in supplement['points']:
                if point.get('id') == previous_id:point['id'] = 'gsi-vector:test'
            supplement_path.write_bytes(gzip.compress(json.dumps(supplement).encode(),mtime=0))
            raw = (json.dumps(points, ensure_ascii=False, separators=(',', ':')) + '\n').encode()
            (source / target.FILE).write_bytes(raw)
            (source / (target.FILE + '.gz')).write_bytes(gzip.compress(raw, mtime=0))
            manifest['uncompressedSizeBytes'] = len(raw)
            vector_source = {'url': 'https://cyberjapandata.gsi.go.jp/xyz/optimal_bvmap-v1/optimal_bvmap-v1.pmtiles',
                             'fetchedAt': '2026-10-13T00:00:00Z', 'sha256': 'test', 'sizeBytes': 100}
            manifest['sourceFiles'] = [vector_source]
            (source / 'manifest.json').write_text(json.dumps(manifest), encoding='utf-8')
            info = target.snapshot(source, staged)
            self.assertEqual(info['sourceSnapshot']['vector']['files'], [vector_source])
            _, release = target.prepare('test-vector', root / 'release', 'dev', staged)
            self.assertEqual(release['sourceSnapshot']['vector']['files'], [vector_source])

    def test_multiple_readings_are_valid_for_release(self):
        point = {"id":"gsi-vector:test", "type":"peak", "name":"白馬岳",
                 "latitude":36.75, "longitude":137.75, "elevationM":2932,
                 "nameReading":["しろうまだけ", "はくばだけ"]}
        target.validate([point])
        for value in ([], ["同じ", "同じ"], [""], [1]):
            with self.subTest(value=value), self.assertRaises(ValueError):
                target.validate([{**point, "nameReading":value}])

    def test_committed_snapshot_and_deterministic_package(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            tag, manifest = target.prepare('test-1', root, 'dev')
            raw = (root / target.FILE).read_bytes()
            self.assertEqual(gzip.decompress((root / (target.FILE + '.gz')).read_bytes()), raw)
            self.assertEqual(manifest['metadata']['pointCount'], len(json.loads(raw)['points']))
            self.assertEqual(manifest['inputSha256'], hashlib.sha256((target.DATA / target.SOURCE_FILE).read_bytes()).hexdigest())
            self.assertEqual(tag, 'gsi-peaks-dev-test-1')
            self.assertIn('/gsi-peaks-dev-test-1/', manifest['downloadUrl'])
            self.assertIn('国土地理院', metadata_maps(manifest['metadata'])["sources"]['gsi-peaks']['attribution'])
            again=target.prepare('test-1', root, 'dev')[1]
            self.assertRegex(again.pop('dataTimestamp'), r'^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$')
            self.assertEqual({k:v for k,v in manifest.items() if k != 'dataTimestamp'}, again)
            stable_tag, stable = target.prepare('test-1', root, 'stable')
            self.assertEqual(stable_tag, 'gsi-peaks-test-1')
            self.assertIn('/gsi-peaks-test-1/', stable['downloadUrl'])

    def test_snapshot_and_mismatched_data(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source, staged = root / 'generated', root / 'staged'
            _, manifest = target.prepare('test-1', source, 'dev')
            manifest['sourceFiles'] = [{'url': metadata_maps(manifest['metadata'])["sources"]['gsi-peaks']['source'], 'fetchedAt': '2026-10-12T04:00:00Z'}]
            (source / 'manifest.json').write_text(json.dumps(manifest), encoding='utf-8')
            target.snapshot(source, staged)
            target.prepare('test-2', root / 'release', 'dev', staged)
            changed = json.loads((staged / target.SOURCE_FILE).read_bytes())
            changed['points'][0]['name'] += '変更'
            (staged / target.SOURCE_FILE).write_text(json.dumps(changed), encoding='utf-8')
            with self.assertRaisesRegex(ValueError, '出典情報が一致しません'):
                target.prepare('test-2', root / 'release', 'dev', staged)

    def test_prepare_needs_only_original_snapshot_and_supplements(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);source=root/'source';source.mkdir()
            for name in (target.SOURCE_FILE,target.INFO,'supplements.json'):
                shutil.copyfile(target.DATA/name,source/name)
            self.assertFalse((source/target.FILE).exists())
            supplements=json.loads((source/'supplements.json').read_bytes())
            supplements['points'][0]['tags'].append('追加タグ')
            (source/'supplements.json').write_text(json.dumps(supplements),encoding='utf-8')
            target.prepare('test-source-only',root/'dist','dev',source)
            generated=json.loads((root/'dist'/target.FILE).read_bytes())
            matching=next(p for p in generated['points'] if p['id']==supplements['points'][0]['id'])
            self.assertIn('追加タグ',matching['tags'])
            self.assertFalse((source/target.FILE).exists())

    def test_invalid_gsi_rows(self):
        row = json.loads((target.DATA / target.SOURCE_FILE).read_bytes())['points'][0]
        for invalid in ({**row, 'id': 'osm:1'}, {**row, 'elevationM': True},
                        {**row, 'elevationM': float('nan')}, {**row, 'aliases': [1]}):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                target.validate([invalid])

    def test_invalid_version_or_branch_never_publishes(self):
        with tempfile.TemporaryDirectory() as folder:
            with self.assertRaises(ValueError):
                target.prepare('../bad', Path(folder), 'dev')
            with patch.dict(os.environ, {'GITHUB_REF': 'refs/heads/topic'}), patch.object(target.release_data, 'gh') as gh:
                with self.assertRaises(ValueError):
                    target.publish('test-1', Path(folder))
                gh.assert_not_called()

    def test_uploaded_files_are_checked_before_publishing(self):
        for branch in ('main', 'dev'):
            for corrupt in (False, True):
                with self.subTest(branch=branch, corrupt=corrupt), tempfile.TemporaryDirectory() as folder:
                    root = Path(folder)
                    environment = {'GH_REPO': 'owner/repo', 'GITHUB_REF': 'refs/heads/' + branch,
                                   'GITHUB_SHA': 'abc', 'GITHUB_RUN_ID': '123'}
                    def gh(*args):
                        if args[:2] == ('release', 'download'):
                            checked = Path(args[args.index('--dir') + 1])
                            checked.mkdir()
                            from scripts.distribution_parts import filenames
                            for name in (target.FILE, target.FILE + '.gz', 'manifest.json', target.INFO, *filenames('gsi-peaks')):
                                shutil.copyfile(root / name, checked / name)
                            if corrupt:
                                (checked / target.FILE).write_bytes(b'corrupt')
                        return ''
                    with patch.dict(os.environ, environment), patch.object(target.release_data, 'gh', side_effect=gh) as calls, patch.object(target.release_data, 'append_summary'):
                        if corrupt:
                            with self.assertRaisesRegex(ValueError, '一致しません'):
                                target.publish('test-1', root)
                        else:
                            target.publish('test-1', root)
                        create = calls.call_args_list[0].args
                        self.assertEqual('--prerelease' in create, branch == 'dev')
                        self.assertIn('--latest=false', create)
                        edits = [call for call in calls.call_args_list if call.args[:2] == ('release', 'edit')]
                        self.assertEqual(len(edits), 0 if corrupt else 1)


if __name__ == '__main__':
    unittest.main()
