"""保存余白を0 mにした独立生成・接合試験。"""
import os
os.environ['SIBSON_SEAMS_NO_OVERLAP']='1'
import generate_sibson_independent_seams as generator

if __name__=='__main__':
 generator.main()
