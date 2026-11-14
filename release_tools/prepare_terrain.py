"""地形SQLiteの下書きReleaseを加工・検査する。"""
import argparse
from release_tools.prepare_surface import prepare

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target",required=True)
    prepare(parser.parse_args().target,kind="terrain")

if __name__=="__main__":
    main()
