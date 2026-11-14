import json
import os
from pathlib import Path
import unittest
from unittest.mock import patch
from release_tools import prepare_surface as target

class TerrainReleaseTests(unittest.TestCase):
    def test_terrain_configuration_is_separate_from_surface(self):
        result=target.resolve_config("gsi-terrain-20261113T000000Z",kind="terrain")
        self.assertEqual(result[0],"gsi_dem")
        self.assertEqual(result[2]["datasetId"],"gsi-terrain-japan")
        self.assertEqual([s["fileName"] for s in result[3]["splitPackages"]],[f"gsi-terrain-z{z}.sqlite3" for z in (9,10,11)])
        with self.assertRaises(ValueError):
            target.resolve_config("gsi-terrain-20261113T000000Z")

    def test_preparation_passes_terrain_kind(self):
        release=dict(tag_name="gsi-terrain-20261113T000000Z-12345-1",draft=True,prerelease=False,target_commitish="a"*40)
        manifest=dict(schemaFormat="yamamuki-manifest",schemaVersion=6,dataSchemaFormat="yamamuki-sqlite",dataSchemaVersion=1,fileName="gsi-terrain-z12.sqlite3",sha256="b"*64,version="20261113T000000Z-12345-1",datasets=[dict(id="gsi-terrain-japan",kind="terrain")])
        def run(args,**kwargs):
            if args[0]=="gh":
                (Path(args[-1])/"gsi-terrain-z12.manifest.json").write_text(json.dumps(manifest),encoding="utf-8")
        with patch.dict(os.environ,GH_REPO="owner/repo",GITHUB_ACTIONS="true",GITHUB_REF="refs/heads/main"),patch.object(target,"find_release",return_value=release),patch.object(target.subprocess,"run",side_effect=run) as calls:
            target.prepare(release["tag_name"],kind="terrain")
            args=calls.call_args.args[0]
            self.assertEqual(args[args.index("-Kind")+1],"terrain")

if __name__=="__main__":
    unittest.main()
