"""NOAA の式で日時から太陽赤緯とグリニッジ時角を求める。"""
import math
from datetime import datetime, timezone
import re


def ephemeris_jst(value):
    if not isinstance(value,str) or not re.fullmatch(r'\d{4}-\d{2}-\d{2}T\d{2}:\d{2}',value):
        raise ValueError('日時は YYYY-MM-DDTHH:mm の JST で指定してください')
    date=datetime.fromisoformat(value+':00+09:00').astimezone(timezone.utc)
    if not 1901<=int(value[:4])<=2099:
        raise ValueError('日時は 1901〜2099 年で指定してください')
    t=(date.timestamp()/86400+2440587.5-2451545)/36525
    r=math.pi/180
    l=(280.46646+t*(36000.76983+t*.0003032))%360
    m=(357.52911+t*(35999.05029-.0001537*t))*r
    e=.016708634-t*(.000042037+.0000001267*t)
    c=math.sin(m)*(1.914602-t*(.004817+.000014*t))+math.sin(2*m)*(.019993-.000101*t)+math.sin(3*m)*.000289
    omega=(125.04-1934.136*t)*r
    lam=(l+c-.00569-.00478*math.sin(omega))*r
    ob=(23+(26+(21.448-t*(46.815+t*(.00059-t*.001813)))/60)/60+.00256*math.cos(omega))*r
    decl=math.asin(math.sin(ob)*math.sin(lam))
    y=math.tan(ob/2)**2
    L=l*r
    eq=4/r*(y*math.sin(2*L)-2*e*math.sin(m)+4*e*y*math.sin(m)*math.cos(2*L)-.5*y*y*math.sin(4*L)-1.25*e*e*math.sin(2*m))
    hour=((date.hour*60+date.minute+date.second/60+eq)/4-180)*r
    return decl,hour
