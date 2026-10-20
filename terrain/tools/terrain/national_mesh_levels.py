"""全国独立LODの定義。番号はタイル一辺kmの2の対数。"""
ERRORS=[2,5,10,25,50,100,200,400]
def definitions():
 return [dict(level=level,name=f'L{level}',maxErrorMeters=error,widthMeters=1000*2**level,minVertices=128,maxVertices=1024) for level,error in enumerate(ERRORS)]
