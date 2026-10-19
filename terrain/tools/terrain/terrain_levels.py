"""地形各レイヤーに共通する「タイル一辺km＋品質」の識別子。"""
SPECS=[('L16_3',1000,16000,3),('L16_2',500,16000,2),('L16_1',250,16000,1),('L8_2',100,8000,2),('L8_1',50,8000,1),('L4_2',30,4000,2),('L4_1',20,4000,1),('L2_2',10,2000,2),('L2_1',5,2000,1),('L1_2',2,1000,2)]
def definitions():
    return [dict(level=name,maxErrorMeters=error,widthMeters=width,quality=quality,areaKm2=(width/1000)**2,minVertexDensityPerKm2=None,maxVerticesPerTile=None) for name,error,width,quality in SPECS]
def level_for_error(error):
    return next(r['level'] for r in definitions() if r['maxErrorMeters']==error)
def update_manifest(manifest):
    levels=definitions()
    manifest['terrainLevels']=levels
    manifest['meshLevels']=levels
    manifest['levelNaming']='L{tileSideKm}_{quality}; mesh, heightmap, normalmap; smaller quality number is finer within the same tile size'
    for meta in manifest['regions'].values():
        for error,record in meta.get('adaptiveSpatial',{}).items():record['level']=level_for_error(int(error))
