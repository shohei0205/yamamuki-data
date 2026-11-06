"""Compare land/water changes at a target display resolution using exact pixel coverage."""
import argparse
import gzip
import json
import math
from pathlib import Path

import numpy as np
import shapely
from shapely.geometry import Point, box, shape


def water(path):
    with gzip.open(path) if path.suffix == ".gz" else path.open("rb") as stream:
        features = json.load(stream)["features"]
    result = shapely.union_all([
        shape(feature["geometry"]) for feature in features
        if feature["properties"]["class"] in (3, 4)
    ])
    if not result.is_valid:
        raise ValueError(f"Invalid water geometry: {path}")
    return result


def pixel_coverage(geometry, grid, cell, minimum=1e-6, sparse=False):
    """Return the fraction covered in each touched grid cell."""
    fractions = {} if sparse else np.zeros(grid * grid)
    for component in shapely.get_parts(geometry):
        if component.is_empty or component.area < minimum:
            continue
        left, bottom, right, top = component.bounds
        xs = np.arange(max(0, math.floor(left / cell)), min(grid, math.ceil(right / cell)))
        ys = np.arange(max(0, math.floor(bottom / cell)), min(grid, math.ceil(top / cell)))
        columns, rows = (values.ravel() for values in np.meshgrid(xs, ys))
        x, y = columns * cell, rows * cell
        pixels = shapely.box(x, y, x + cell, y + cell)
        area = shapely.area(shapely.intersection(component, pixels))
        indices = rows * grid + columns
        if sparse:
            for index, value in zip(indices[area > 0], area[area > 0]):
                fractions[index] = fractions.get(index, 0.) + value / (cell * cell)
        else:
            fractions[indices] += area / (cell * cell)
    return fractions


def change_summary(change, fractions):
    return {
        "areaProjectedM2": round(change.area, 2),
        "affectedPixels": int(np.count_nonzero(fractions > 1e-10)),
        "maximumPixelCoverage": round(float(fractions.max(initial=0.)), 6),
        "pixelsAtLeastHalfCovered": int(np.count_nonzero(fractions >= 0.5)),
        "pixelsFullyCovered": int(np.count_nonzero(fractions >= 1 - 1e-9)),
    }


def water_pixel_areas(geometry, changed, grid, cell):
    columns, rows = changed % grid, changed // grid
    areas = np.zeros(len(changed))
    block_side = 64
    blocks_per_row = (grid + block_side - 1) // block_side
    blocks = (rows // block_side) * blocks_per_row + columns // block_side
    order = np.argsort(blocks)
    splits = np.flatnonzero(np.diff(blocks[order])) + 1
    for indices in np.split(order, splits):
        if not len(indices):
            continue
        x, y = columns[indices] * cell, rows[indices] * cell
        local = geometry.intersection(box(x.min(), y.min(), x.max() + cell, y.max() + cell))
        pixels = shapely.box(x, y, x + cell, y + cell)
        areas[indices] = shapely.area(shapely.intersection(local, pixels))
    return areas


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("before", type=Path)
    parser.add_argument("after", type=Path)
    parser.add_argument("--grid", type=int, required=True)
    parser.add_argument("--side", type=float, default=256000)
    parser.add_argument("--circle", type=float, nargs=3, metavar=("X", "Y", "RADIUS"))
    parser.add_argument("--require-filled", type=float, nargs=2, metavar=("X", "Y"),
                        help="Require this point to change from water to land.")
    parser.add_argument("--require-no-majority-change", action="store_true")
    args = parser.parse_args()
    if args.grid < 1 or not math.isfinite(args.side) or args.side <= 0:
        parser.error("grid and side must be positive")
    cell = args.side / args.grid
    bounds = box(0, 0, args.side, args.side)
    if args.circle:
        x, y, radius = args.circle
        if radius <= 0 or not all(map(math.isfinite, args.circle)):
            parser.error("circle needs finite coordinates and a positive radius")
        bounds = bounds.intersection(Point(x, y).buffer(radius, quad_segs=64))
    before, after = (water(path).intersection(bounds) for path in (args.before, args.after))
    land_to_water = after.difference(before)
    water_to_land = before.difference(after)
    gained, lost = (pixel_coverage(g, args.grid, cell, sparse=True) for g in (land_to_water, water_to_land))
    changed = np.array(sorted(gained.keys() | lost.keys()), dtype=np.int64)
    gains = np.array(list(gained.values()))
    losses = np.array(list(lost.values()))
    # Assess majority against the full pixel, even when the region of interest clips it.
    full_before, full_after = (water(path) for path in (args.before, args.after)) if args.circle else (before, after)
    if args.require_filled:
        pin = Point(*args.require_filled)
        if not full_before.covers(pin) or full_after.covers(pin):
            parser.exit(1, "The specified point did not change from water to land.\n")
    old_area = water_pixel_areas(full_before, changed, args.grid, cell)
    new_area = water_pixel_areas(full_after, changed, args.grid, cell)
    majority_changed = np.count_nonzero((old_area >= cell * cell / 2) != (new_area >= cell * cell / 2))
    print(json.dumps({
        "grid": [args.grid, args.grid],
        "pixelProjectedM": cell,
        "majorityClassChangedPixels": int(majority_changed),
        "landToWater": change_summary(land_to_water, gains),
        "waterToLand": change_summary(water_to_land, losses),
    }, indent=2))
    if args.require_no_majority_change and majority_changed:
        parser.exit(1, "Land/water majority changed.\n")


if __name__ == "__main__":
    main()
