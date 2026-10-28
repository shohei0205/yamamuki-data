"""共有Voronoi辺の長さに基づくLaplace自然近傍補間。"""
from sibson_surface import SibsonSurface

class LaplaceSurface(SibsonSurface):
 def __init__(self,points):
  super().__init__(points,library="build/laplace_eval.dll")
