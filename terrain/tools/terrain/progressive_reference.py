"""既存の比較品質を変えずに16,384頂点の参考メッシュを生成する。"""
import progressive_tin_compare as compare

if __name__ == '__main__':
    compare.BUDGETS = [16384]
    compare.OUTPUT = compare.ROOT / 'build/progressive-tin-reference.sqlite3'
    compare.generate()
