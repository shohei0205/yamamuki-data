"""余白0 mに加え、タイルの外周の誤差も評価する。"""
import os
os.environ['SIBSON_SEAMS_NO_OVERLAP']='1'
os.environ['SIBSON_SEAMS_BOUNDARY_AUDIT']='1'
import generate_sibson_independent_seams as generator

if __name__=='__main__':
 generator.main()
