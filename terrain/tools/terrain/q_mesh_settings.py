"""全国Q4～Q12の合意設定。保存単位と品質を旧L番号から分離する。"""
import math

def definitions():
    result=[]
    for q in range(4,13):
        width=2000*2**(q-4);radius=width/2;area=math.pi*(radius/1000)**2
        result.append(dict(level=q,name=f'Q{q}',maxErrorMeters=12.5*2**(q-4),widthMeters=width,
            sampleSpacingMeters=31.25,minimumAngleDegrees=10,additionalFraction=.2,
            densityRadiusMeters=radius,densityReferenceVertices=4096,
            densityReferenceVerticesPerKm2=4096/area,densityReferenceAreaKm2=area))
    return result

def nearby_density_ratio(vertices,level):
    """タイル境界を越えた円内の共有点を一度だけ数えた値を受け取る。"""
    density=vertices/level['densityReferenceAreaKm2']
    return density/level['densityReferenceVerticesPerKm2']
