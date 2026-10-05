#!/usr/bin/env python3
"""Locally smooth GLB surface partitions without changing the source geometry.

Python standard library only. This produces geometric surface regions, not a
mechanical disassembly model. Inputs are read-only; an output must be a new path.
"""

import argparse
from collections import Counter, defaultdict, deque
import copy
import hashlib
import itertools
import json
import math
from pathlib import Path
import struct
import sys


COMPONENTS = {5120: "b", 5121: "B", 5122: "h", 5123: "H", 5125: "I", 5126: "f"}
WIDTHS = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def distance(a, b):
    return math.sqrt(sum((x - y) ** 2 for x, y in zip(a, b)))


def bbox(points):
    lo = [min(p[a] for p in points) for a in range(3)]
    hi = [max(p[a] for p in points) for a in range(3)]
    return {"min": lo, "max": hi}


class Glb:
    def __init__(self, path=None, raw=None):
        self.raw = Path(path).read_bytes() if raw is None else raw
        self.sha256 = hashlib.sha256(self.raw).hexdigest()
        require(len(self.raw) >= 20, "GLB header is truncated")
        magic, version, length = struct.unpack_from("<III", self.raw)
        require(magic == 0x46546C67 and version == 2 and length == len(self.raw), "Invalid GLB header")
        self.bin = None
        self.json = None
        offset = 12
        kinds = []
        while offset < length:
            size, kind = struct.unpack_from("<II", self.raw, offset)
            require(size % 4 == 0 and offset + 8 + size <= length, "Invalid GLB chunk range")
            value = self.raw[offset + 8:offset + 8 + size]
            kinds.append(kind)
            if kind == 0x4E4F534A:
                self.json = json.loads(value)
            elif kind == 0x004E4942:
                self.bin = value
            offset += size + 8
        require(kinds == [0x4E4F534A, 0x004E4942], "Only self-contained JSON+BIN GLB is supported")
        require(self.json.get("asset", {}).get("version") == "2.0", "glTF version must be 2.0")
        require(not self.json.get("extensionsRequired"), "Required extensions are not supported")
        require(not self.json.get("animations") and not self.json.get("skins"), "Animated/skinned inputs are not supported")
        require(len(self.json.get("buffers", [])) == 1 and "uri" not in self.json["buffers"][0], "A single embedded buffer is required")

    def accessor(self, index):
        a = self.json["accessors"][index]
        require("sparse" not in a and a["type"] in WIDTHS, "Sparse/matrix accessors are not supported")
        view = self.json["bufferViews"][a["bufferView"]]
        require(view["buffer"] == 0 and a["componentType"] in COMPONENTS, "Unsupported accessor buffer/type")
        parser = struct.Struct("<" + COMPONENTS[a["componentType"]] * WIDTHS[a["type"]])
        base = view.get("byteOffset", 0) + a.get("byteOffset", 0)
        stride = view.get("byteStride", parser.size)
        require(a["count"] > 0 and stride >= parser.size, "Empty/invalid accessor")
        require(base + (a["count"] - 1) * stride + parser.size <= view.get("byteOffset", 0) + view["byteLength"] <= len(self.bin), "Accessor exceeds its bufferView")
        return [parser.unpack_from(self.bin, base + i * stride) for i in range(a["count"])]


class Geometry:
    def __init__(self, glb):
        self.glb = glb
        g = glb.json
        self.nodes = g["nodes"]
        require(set(g["scenes"][g.get("scene", 0)]["nodes"]) == set(range(len(self.nodes))), "Inputs must have flat scene-root nodes")
        require(all(set(n) <= {"mesh", "name", "translation", "extras"} for n in self.nodes), "Only flat translation-only mesh nodes are supported")
        self.points = []
        self.triangles = []
        self.face_sources = []
        self.face_nodes = []
        self.primitives = []
        self.names = [n.get("name", f"part_{i}") for i, n in enumerate(self.nodes)]
        require(len(set(self.names)) == len(self.names), "Node names must be unique")
        for node_index, node in enumerate(self.nodes):
            translation = node.get("translation", [0, 0, 0])
            require(len(translation) == 3 and all(math.isfinite(x) for x in translation), "Invalid node translation")
            mesh = g["meshes"][node["mesh"]]
            for primitive in mesh["primitives"]:
                require(primitive.get("mode", 4) == 4 and not primitive.get("targets"), "Only triangle primitives without morph targets are supported")
                attributes = {name: glb.accessor(index) for name, index in primitive["attributes"].items()}
                require("POSITION" in attributes, "Primitive has no POSITION")
                position_accessor = g["accessors"][primitive["attributes"]["POSITION"]]
                require(position_accessor["componentType"] == 5126 and position_accessor["type"] == "VEC3", "POSITION must be FLOAT VEC3")
                positions = attributes["POSITION"]
                require(all(len(values) == len(positions) for values in attributes.values()), "Attribute vertex counts differ")
                require(all(math.isfinite(x) for p in positions for x in p), "Nonfinite POSITION")
                points = [tuple(p[a] + translation[a] for a in range(3)) for p in positions]
                if "indices" in primitive:
                    require(g["accessors"][primitive["indices"]]["type"] == "SCALAR", "Indices must be SCALAR")
                    indices = [x[0] for x in glb.accessor(primitive["indices"])]
                else:
                    indices = list(range(len(positions)))
                require(len(indices) % 3 == 0 and indices and all(0 <= x < len(points) for x in indices), "Invalid triangle indices")
                offset = len(self.points)
                primitive_index = len(self.primitives)
                self.primitives.append({"original": primitive, "attributes": attributes, "world": points})
                self.points.extend(points)
                for i in range(0, len(indices), 3):
                    t = tuple(indices[i:i + 3])
                    self.triangles.append(tuple(offset + v for v in t))
                    self.face_sources.append((primitive_index, t))
                    self.face_nodes.append(node_index)
        self.bounds = bbox(self.points)


def centroid(points):
    return tuple(sum(p[a] for p in points) / len(points) for a in range(3))


def align_labels(whole, initial, tolerance):
    require(len(whole.triangles) == len(initial.triangles), "Initial partitions do not cover the same number of triangles")
    exact = {}
    canonical = []
    vertex_grid = defaultdict(list)
    for p in whole.points:
        if p not in exact:
            i = len(canonical)
            exact[p] = i
            canonical.append(p)
            vertex_grid[tuple(math.floor(x / tolerance) for x in p)].append(i)
    nearest = []
    max_error = 0
    for p in initial.points:
        cell = tuple(math.floor(x / tolerance) for x in p)
        candidates = [i for delta in itertools.product((-1, 0, 1), repeat=3)
                      for i in vertex_grid.get(tuple(cell[a] + delta[a] for a in range(3)), [])]
        require(candidates, "Initial vertex is not on the original surface")
        i = min(candidates, key=lambda v: sum((p[a] - canonical[v][a]) ** 2 for a in range(3)))
        error = distance(p, canonical[i])
        require(error <= tolerance, "Initial vertex differs from original surface beyond tolerance")
        nearest.append(i)
        max_error = max(max_error, error)
    keys = defaultdict(list)
    centroid_grid = defaultdict(list)
    for face, t in enumerate(whole.triangles):
        keys[tuple(sorted(exact[whole.points[v]] for v in t))].append(face)
        c = centroid([whole.points[v] for v in t])
        centroid_grid[tuple(math.floor(x / tolerance) for x in c)].append(face)
    labels = [-1] * len(whole.triangles)
    fallback = 0
    for t, label in zip(initial.triangles, initial.face_nodes):
        key = tuple(sorted(nearest[v] for v in t))
        candidates = [f for f in keys.get(key, []) if labels[f] == -1]
        if candidates:
            f = candidates[0]
        else:
            fallback += 1
            p = [initial.points[v] for v in t]
            c = centroid(p)
            cell = tuple(math.floor(x / tolerance) for x in c)
            candidates = [f for delta in itertools.product((-1, 0, 1), repeat=3)
                          for f in centroid_grid.get(tuple(cell[a] + delta[a] for a in range(3)), [])
                          if labels[f] == -1]
            matches = []
            for f in candidates:
                original = [whole.points[v] for v in whole.triangles[f]]
                error = min(max(distance(a, b) for a, b in zip(p, order))
                            for order in itertools.permutations(original))
                if error <= tolerance:
                    matches.append((error, f))
            require(matches, "Initial triangle has no unused matching original triangle")
            _, f = min(matches)
        labels[f] = label
    require(all(x >= 0 for x in labels), "Original triangles are missing from initial partitions")
    return labels, {"maximumVertexError": max_error, "toleranceFallbackTriangles": fallback, "allOriginalFacesMatchedOnce": True}


def face_graph(whole):
    welded = {p: i for i, p in enumerate(dict.fromkeys(whole.points))}
    edges = defaultdict(list)
    normals = []
    for face, t in enumerate(whole.triangles):
        p, q, r = [whole.points[v] for v in t]
        a = [q[i] - p[i] for i in range(3)]
        b = [r[i] - p[i] for i in range(3)]
        n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
        length = math.sqrt(sum(x * x for x in n))
        require(length > 0, "Original geometry contains a zero-area triangle")
        normals.append(tuple(x / length for x in n))
        v = [welded[whole.points[i]] for i in t]
        for x, y in ((v[0], v[1]), (v[1], v[2]), (v[2], v[0])):
            require(x != y, "Original geometry has a collapsed edge")
            edges[tuple(sorted((x, y)))].append(face)
    canonical = list(welded)
    adjacency = [dict() for _ in whole.triangles]
    for (a, b), faces in edges.items():
        length = distance(canonical[a], canonical[b])
        for x, y in itertools.combinations(set(faces), 2):
            dot = max(0.0, min(1.0, sum(normals[x][i] * normals[y][i] for i in range(3))))
            weight = length * (0.05 + 0.95 * dot ** 4)
            adjacency[x][y] = adjacency[x].get(y, 0) + weight
            adjacency[y][x] = adjacency[y].get(x, 0) + weight
    return adjacency


def connectivity(labels, adjacency, count):
    regions = [set() for _ in range(count)]
    for f, label in enumerate(labels):
        regions[label].add(f)
    components = []
    for faces in regions:
        remaining = set(faces)
        groups = 0
        while remaining:
            groups += 1
            stack = [remaining.pop()]
            while stack:
                f = stack.pop()
                for n in adjacency[f]:
                    if n in remaining:
                        remaining.remove(n)
                        stack.append(n)
        components.append(groups)
    return components


def removable(face, labels, adjacency):
    label = labels[face]
    targets = {n for n in adjacency[face] if labels[n] == label}
    if len(targets) <= 1:
        return True
    start = targets.pop()
    seen = {face, start}
    pending = deque([start])
    while pending and targets:
        f = pending.popleft()
        for n in adjacency[f]:
            if n not in seen and labels[n] == label:
                targets.discard(n)
                seen.add(n)
                pending.append(n)
    return not targets


def boundary_cost(labels, adjacency):
    return sum(w for f, neighbors in enumerate(adjacency) for n, w in neighbors.items()
               if f < n and labels[f] != labels[n])


def optimize(initial, adjacency, count, protected, passes, drift):
    labels = initial[:]
    sizes = Counter(labels)
    initial_sizes = sizes.copy()
    lower = {i: max(1, math.ceil(initial_sizes[i] * (1 - drift))) for i in range(count)}
    upper = {i: max(initial_sizes[i], math.floor(initial_sizes[i] * (1 + drift))) for i in range(count)}
    before = boundary_cost(labels, adjacency)
    moved = []
    rounds = []

    def candidate(f):
        a = labels[f]
        if a in protected or sizes[a] <= lower[a]:
            return None
        possible = {labels[n] for n in adjacency[f]} - protected - {a}
        options = []
        for b in possible:
            if sizes[b] >= upper[b]:
                continue
            delta = sum(w * (int(b != labels[n]) - int(a != labels[n]))
                        for n, w in adjacency[f].items())
            if delta < -1e-12:
                options.append((delta, b))
        return min(options) if options else None

    for round_index in range(passes):
        options = []
        for f in range(len(labels)):
            value = candidate(f)
            if value is not None:
                options.append((value[0], f))
        changes = 0
        for _, f in sorted(options):
            value = candidate(f)
            if value is None or not removable(f, labels, adjacency):
                continue
            delta, b = value
            a = labels[f]
            labels[f] = b
            sizes[a] -= 1
            sizes[b] += 1
            moved.append({"face": f, "from": a, "to": b, "weightedCostDelta": delta})
            changes += 1
        rounds.append({"pass": round_index + 1, "moves": changes, "weightedBoundaryCost": boundary_cost(labels, adjacency)})
        if not changes:
            break
    after = boundary_cost(labels, adjacency)
    changed = [f for f, (a, b) in enumerate(zip(initial, labels)) if a != b]
    require(changed and after < before, "No genuine surface partition improvement was possible with these constraints")
    require(connectivity(labels, adjacency, count) == [1] * count, "Optimization produced disconnected/empty regions")
    require(all(initial[f] not in protected and labels[f] not in protected for f in changed), "Protected region changed")
    return labels, {"weightedBoundaryCostBefore": before, "weightedBoundaryCostAfter": after,
                    "weightedBoundaryCostReductionPercent": (before - after) * 100 / before,
                    "changedOriginalFaces": len(changed), "moveOperations": len(moved),
                    "passes": rounds, "initialFaceCounts": [initial_sizes[i] for i in range(count)],
                    "finalFaceCounts": [sizes[i] for i in range(count)],
                    "maximumFaceCountDriftFraction": drift}, moved


def export_glb(whole, labels, names):
    source = whole.glb.json
    target = {"asset": {"version": "2.0", "generator": "everything-manual local surface resegmentation v1"},
              "scene": 0, "scenes": [{"name": "Locally resegmented surface", "nodes": list(range(len(names)))}],
              "nodes": [], "meshes": [], "accessors": [], "bufferViews": [], "buffers": [],
              "extras": {"segmentationKind": "external-surface-regions", "mechanicalDisassembly": False}}
    for key in ("materials", "textures", "samplers"):
        if key in source:
            target[key] = copy.deepcopy(source[key])
    data = bytearray()

    def view(raw, kind=None):
        while len(data) % 4:
            data.append(0)
        result = {"buffer": 0, "byteOffset": len(data), "byteLength": len(raw)}
        if kind is not None:
            result["target"] = kind
        target["bufferViews"].append(result)
        data.extend(raw)
        return len(target["bufferViews"]) - 1

    if source.get("images"):
        target["images"] = copy.deepcopy(source["images"])
        for image in target["images"]:
            require("uri" not in image and "bufferView" in image, "Textures must be embedded images")
            v = source["bufferViews"][image["bufferView"]]
            require(v["buffer"] == 0, "Invalid image buffer")
            offset = v.get("byteOffset", 0)
            image["bufferView"] = view(whole.glb.bin[offset:offset + v["byteLength"]])

    def accessor(values, template, kind):
        parser = struct.Struct("<" + COMPONENTS[template["componentType"]] * WIDTHS[template["type"]])
        raw = bytearray(len(values) * parser.size)
        for i, value in enumerate(values):
            parser.pack_into(raw, i * parser.size, *value)
        result = {"bufferView": view(raw, kind), "componentType": template["componentType"],
                  "count": len(values), "type": template["type"]}
        if template.get("normalized"):
            result["normalized"] = True
        # Derive bounds from serialized FLOAT values, including float32 rounding.
        if template["componentType"] == 5126:
            decoded = [parser.unpack_from(raw, i * parser.size) for i in range(len(values))]
            result["min"] = [min(x[a] for x in decoded) for a in range(len(decoded[0]))]
            result["max"] = [max(x[a] for x in decoded) for a in range(len(decoded[0]))]
        target["accessors"].append(result)
        return len(target["accessors"]) - 1

    regions = [defaultdict(list) for _ in names]
    for face, (primitive, t) in enumerate(whole.face_sources):
        regions[labels[face]][primitive].append(t)
    for region, by_primitive in enumerate(regions):
        points = [whole.primitives[p]["world"][v] for p, faces in by_primitive.items() for t in faces for v in t]
        bounds = bbox(points)
        center = [(bounds["min"][a] + bounds["max"][a]) / 2 for a in range(3)]
        result = {"name": f"resegmented_mesh_{region}", "primitives": []}
        for p, faces in sorted(by_primitive.items()):
            primitive = whole.primitives[p]
            original = primitive["original"]
            vertices = list(dict.fromkeys(v for t in faces for v in t))
            remap = {v: i for i, v in enumerate(vertices)}
            attributes = {}
            for name, index in original["attributes"].items():
                template = source["accessors"][index]
                values = [primitive["attributes"][name][v] for v in vertices]
                if name == "POSITION":
                    values = [tuple(primitive["world"][v][a] - center[a] for a in range(3)) for v in vertices]
                attributes[name] = accessor(values, template, 34962)
            indices = [(remap[v],) for t in faces for v in t]
            item = {"attributes": attributes, "indices": accessor(indices, {"componentType": 5125, "type": "SCALAR"}, 34963), "mode": 4}
            if "material" in original:
                item["material"] = original["material"]
            result["primitives"].append(item)
        target["meshes"].append(result)
        target["nodes"].append({"name": names[region], "mesh": region, "translation": center})
    while len(data) % 4:
        data.append(0)
    target["buffers"] = [{"byteLength": len(data)}]
    encoded = json.dumps(target, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode()
    encoded += b" " * (-len(encoded) % 4)
    length = 12 + 8 + len(encoded) + 8 + len(data)
    return struct.pack("<III", 0x46546C67, 2, length) + struct.pack("<II", len(encoded), 0x4E4F534A) + encoded + struct.pack("<II", len(data), 0x004E4942) + data


def verify_attributes(whole, candidate, labels, count, tolerance):
    expected = []
    grouped = [defaultdict(list) for _ in range(count)]
    for face, (primitive, _) in enumerate(whole.face_sources):
        grouped[labels[face]][primitive].append(face)
    for label, group in enumerate(grouped):
        for primitive, faces in sorted(group.items()):
            expected.extend((label, face) for face in faces)
    require(len(expected) == len(candidate.face_sources), "Export changed triangle count")
    maximum_error = 0
    compared_attributes = Counter()
    for index, (label, source_face) in enumerate(expected):
        require(candidate.face_nodes[index] == label, "Export changed node numbering")
        source_primitive, source_vertices = whole.face_sources[source_face]
        target_primitive, target_vertices = candidate.face_sources[index]
        a, b = whole.primitives[source_primitive], candidate.primitives[target_primitive]
        require(a["original"].get("material") == b["original"].get("material"), "Triangle material changed")
        require(set(a["attributes"]) == set(b["attributes"]), "Vertex attribute names changed")
        for name in a["attributes"]:
            old_index = a["original"]["attributes"][name]
            new_index = b["original"]["attributes"][name]
            old_def = whole.glb.json["accessors"][old_index]
            new_def = candidate.glb.json["accessors"][new_index]
            require(all(old_def.get(k) == new_def.get(k) for k in ("componentType", "type", "normalized")), "Vertex attribute format changed")
            for v, w in zip(source_vertices, target_vertices):
                if name == "POSITION":
                    error = distance(a["world"][v], b["world"][w])
                    require(error <= tolerance, "Export changed a world-space vertex")
                    maximum_error = max(maximum_error, error)
                else:
                    require(a["attributes"][name][v] == b["attributes"][name][w], "Export changed a source UV/normal/vertex attribute")
                compared_attributes[name] += 1
    images = []
    require(len(whole.glb.json.get("images", [])) == len(candidate.glb.json.get("images", [])), "Export changed texture count")
    for index, (a, b) in enumerate(zip(whole.glb.json.get("images", []), candidate.glb.json.get("images", []))):
        def image_bytes(glb, image):
            v = glb.json["bufferViews"][image["bufferView"]]
            offset = v.get("byteOffset", 0)
            return glb.bin[offset:offset + v["byteLength"]]
        old, new = image_bytes(whole.glb, a), image_bytes(candidate.glb, b)
        require(old == new and a.get("mimeType") == b.get("mimeType"), "Export changed embedded texture bytes/type")
        images.append({"image": index, "bytes": len(old), "sha256": hashlib.sha256(old).hexdigest(), "identical": True})
    return {"comparedTriangleCorners": len(expected) * 3, "comparedVertexAttributes": dict(compared_attributes),
            "maximumWorldVertexError": maximum_error, "triangleWindingPreserved": True,
            "uvNormalAndOtherAttributesExact": True, "textureImages": images}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--whole", required=True, type=Path)
    parser.add_argument("--initial-parts", required=True, type=Path)
    parser.add_argument("--output", type=Path, help="New GLB path; without it the command is a dry run")
    parser.add_argument("--report-dir", type=Path)
    parser.add_argument("--dry-run", action="store_true", help="Calculate and verify a candidate, without writing a GLB")
    parser.add_argument("--protect", default="tripo_part_0,tripo_part_6,tripo_part_7", help="Comma-separated node names whose complete surface must remain unchanged")
    parser.add_argument("--passes", type=int, default=4)
    parser.add_argument("--size-drift", type=float, default=0.05, help="Maximum per-region triangle-count drift")
    parser.add_argument("--tolerance", type=float, default=1e-6)
    args = parser.parse_args()
    require(1 <= args.passes <= 20 and 0 < args.size_drift <= 0.25 and 0 < args.tolerance <= 1e-4, "Invalid optimization limits")
    if args.output:
        require(args.output.resolve() not in {args.whole.resolve(), args.initial_parts.resolve()}, "Output cannot overwrite an input")
        if not args.dry_run:
            require(not args.output.exists() and not args.output.is_symlink(), "Output path already exists; outputs never overwrite")
    print("Read-only geometry mapping and surface partition optimization…", file=sys.stderr)
    whole = Geometry(Glb(args.whole))
    initial = Geometry(Glb(args.initial_parts))
    protected_names = [s for s in args.protect.split(",") if s]
    require(all(s in initial.names for s in protected_names), "Protected node name is missing")
    protected = {initial.names.index(s) for s in protected_names}
    original, mapping = align_labels(whole, initial, args.tolerance)
    adjacency = face_graph(whole)
    count = len(initial.names)
    require(connectivity(original, adjacency, count) == [1] * count, "Initial welded surface regions must all be nonempty and connected")
    print(f"Matched {len(original)} original faces into {count} connected regions; smoothing boundaries…", file=sys.stderr)
    final, optimization, moves = optimize(original, adjacency, count, protected, args.passes, args.size_drift)
    raw = export_glb(whole, final, initial.names)
    candidate = Geometry(Glb(raw=raw))
    exported_labels, export_mapping = align_labels(whole, candidate, args.tolerance)
    require(exported_labels == final, "Export changed face ownership or dropped/duplicated source geometry")
    require(connectivity(exported_labels, adjacency, count) == [1] * count, "Exported regions must remain connected")
    bounds_error = max(abs(whole.bounds[k][a] - candidate.bounds[k][a]) for k in ("min", "max") for a in range(3))
    require(bounds_error <= args.tolerance, "Export changed overall model bounds")
    material_match = all(whole.glb.json.get(k) == candidate.glb.json.get(k) for k in ("materials", "textures", "samplers"))
    require(material_match, "Export changed PBR material definitions")
    attribute_verification = verify_attributes(whole, candidate, final, count, args.tolerance)
    protected_result = []
    for label in sorted(protected):
        before = [f for f, value in enumerate(original) if value == label]
        after = [f for f, value in enumerate(exported_labels) if value == label]
        require(before == after, "Protected node surface changed")
        protected_result.append({"name": initial.names[label], "originalFaces": len(before), "changedFaces": 0, "identicalSourceFaces": True})
    report = {"status": "passed", "algorithm": "welded-edge curvature-weighted connectivity-preserving boundary optimization v1",
              "segmentationKind": "external-surface-regions", "mechanicalDisassembly": False,
              "dryRun": args.dry_run or not args.output,
              "sourceWholeSha256": whole.glb.sha256, "sourcePartsSha256": initial.glb.sha256,
              "candidateSha256": hashlib.sha256(raw).hexdigest(), "candidateBytes": len(raw),
              "triangles": len(whole.triangles), "regions": count, "nodeNames": initial.names,
              "allRegionsNonemptyAndConnected": True, "allOriginalTrianglesCoveredExactlyOnce": True,
              "worldBounds": candidate.bounds, "maximumBoundsError": bounds_error,
              "sourceMaterialDefinitionsPreserved": material_match, "sourceVertexAttributesPreserved": True,
              "attributeVerification": attribute_verification,
              "mapping": mapping, "exportMapping": export_mapping, "optimization": optimization,
              "protectedRegions": protected_result, "providerRequests": 0, "paidRequests": 0, "databaseWrites": 0}
    if args.output and not args.dry_run:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("xb") as output:
            output.write(raw)
        report["output"] = str(args.output.resolve())
    if args.report_dir:
        args.report_dir.mkdir(parents=True, exist_ok=True)
        (args.report_dir / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
        changed = [{"face": f, "from": a, "to": b} for f, (a, b) in enumerate(zip(original, final)) if a != b]
        (args.report_dir / "changed-faces.json").write_text(json.dumps(changed, separators=(",", ":")) + "\n")
        (args.report_dir / "moves.json").write_text(json.dumps(moves, separators=(",", ":")) + "\n")
        (args.report_dir / "face-labels.json").write_text(json.dumps({"initial": original, "final": final}, separators=(",", ":")) + "\n")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, struct.error) as error:
        print(f"resegmentation rejected: {error}", file=sys.stderr)
        sys.exit(1)
