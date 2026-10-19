"""既存品質を変更せず128・256頂点の比較メッシュを追加する。"""
import progressive_tin_compare as compare
if __name__=='__main__':
    compare.BUDGETS=[128,256]
    compare.BOUNDARY_ERROR=100
    compare.OUTPUT=compare.ROOT/'build/progressive-tin-coarse.sqlite3'
    compare.generate()
