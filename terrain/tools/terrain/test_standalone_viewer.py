import unittest
from build_standalone_viewer import factory,js_module

class StandaloneBundleTest(unittest.TestCase):
 def test_exports_stay_in_module_scope(self):
  result=js_module("import * as X from './x.js';\nexport * from './x.js';\nexport const A=3;\nexport class B {}",'const X={};','...X,')
  self.assertNotIn('import ',result)
  self.assertNotIn('export ',result)
  self.assertIn('return {...X,A,B}',result)
 def test_unknown_exports_are_rejected(self):
  with self.assertRaises(ValueError):js_module('export default 3;')
 def test_factory_uses_embedded_binary(self):
  result=factory('var Module=()=>import.meta.url;\nexport default Module;')
  self.assertNotIn('import.meta',result)
  self.assertIn('return Module;',result)
  with self.assertRaises(ValueError):factory('export default function(){}')

if __name__=='__main__':unittest.main()
