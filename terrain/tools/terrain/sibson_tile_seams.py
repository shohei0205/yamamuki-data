"""自然近傍の影響範囲を保った分割と、複数曲面の接合試験。"""
import numpy as np
from sibson_surface import SibsonSurface

MAX_WIDTH=2000.
STEP=31.25
TILES={
 'nw':[-8000,0,0,8000], 'ne':[0,0,8000,8000],
 'sw':[-8000,-8000,0,0], 'se':[0,-8000,8000,0],
 'large':[-16000,-8000,0,8000],
}


def expanded(bounds,width=MAX_WIDTH):
    x,y,u,v=bounds
    return [max(-16000,x-width),max(-16000,y-width),min(16000,u+width),min(16000,v+width)]


def extract_points(model,rect):
    """照会矩形に外接円が届く面と、その頂点の隣接点を保存する。"""
    centers=model.circles[:,:2]
    closest=np.clip(centers,np.asarray(rect[:2]),np.asarray(rect[2:]))
    hit=np.sum((centers-closest)**2,axis=1)<=model.circles[:,2]*(1+1e-12)+1e-8
    ids=np.unique(model.tri[hit])
    neighbors=[model.links[model.off[i]:model.off[i+1]] for i in ids]
    ids=np.unique(np.concatenate([ids,*neighbors]))
    return np.column_stack((model.xy[ids],model.z[ids])).astype('<f4')


def smooth(t):
    s=np.clip(t,0,1)
    return s*s*s*(10+s*(-15+6*s)),np.where((t>0)&(t<1),30*s*s*(s-1)**2,0.)


def weight(query,bounds,width):
    if width<=0:raise ValueError('接合幅は正にしてください')
    q=np.asarray(query);x,y,u,v=bounds
    a,da=smooth((q[:,0]-x+width)/(2*width));b,db=smooth((u+width-q[:,0])/(2*width))
    c,dc=smooth((q[:,1]-y+width)/(2*width));d,dd=smooth((v+width-q[:,1])/(2*width))
    return a*b*c*d,np.column_stack(((da*b-a*db)*c*d,a*b*(dc*d-c*dd)))/(2*width)


def blend(query,tiles,width=500.):
    """同じ座標・同じ読込集合に対し、所有タイルによらない高さと傾きを返す。"""
    q=np.asarray(query);n=len(q);total=np.zeros(n);numerator=np.zeros(n)
    dtotal=np.zeros((n,2));dnumerator=np.zeros((n,2))
    for bounds,evaluate in tiles:
        w,dw=weight(q,bounds,width);active=w>0
        if not active.any():continue
        values=evaluate(q[active]);h=values[:,0];g=values[:,1:]
        total[active]+=w[active];numerator[active]+=w[active]*h
        dtotal[active]+=dw[active];dnumerator[active]+=w[active,None]*g+dw[active]*h[:,None]
    if np.any(total<=0):raise ValueError('読込タイルの範囲外です')
    height=numerator/total
    gradient=(dnumerator-dtotal*height[:,None])/total[:,None]
    return np.column_stack((height,gradient))


def evaluator(model):
    def evaluate(q):
        h=model(q);gradient=[]
        for axis in range(2):
            shift=np.zeros(2);shift[axis]=1.
            gradient.append((model(q+shift)-model(q-shift))/2)
        return np.column_stack((h,*gradient))
    return evaluate
