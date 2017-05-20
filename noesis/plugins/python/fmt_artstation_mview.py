import noesis, rapi, json, io, os
from inc_noesis import *
from pprint import pprint

def registerNoesisTypes():
    handle = noesis.register("Marmoset View", ".mview")
    noesis.setHandlerTypeCheck(handle, checkType)
    noesis.setHandlerLoadModel(handle, loadModel)

    # noesis.logPopup()
    return 1

def checkType(data):
    return 1

def loadModel(data, mdlList):
    bs = NoeBitStream(data)

    files = extract(bs)
    meshes = []
    texList = []
    matList = []

    # load the scene
    fscene = find(files["application/json"], "filename", "scene.json")["data"]
    scene = json.loads(fscene.decode("cp1252"))

    # load all the materials
    for mat in scene["materials"]:
        name = mat["name"]

        fdiffuse = mat["albedoTex"]
        diffuse = loadTex(files, fdiffuse)
        texList.append(diffuse)
        material = NoeMaterial(name, fdiffuse)

        if "normalTex" in mat:
            fnormal = mat["normalTex"]
            normal =  loadTex(files, fnormal)
            texList.append(normal)
            material.setNormalTexture(fnormal)

        if "reflectivityTex" in mat:
            fspecular = mat["reflectivityTex"]
            specular = loadTex(files, fspecular)
            texList.append(specular)
            material.setSpecularTexture(fspecular)

        if "alphaTest" in mat:
            material.setAlphaTest(mat["alphaTest"])

        matList.append(material)

    # load all the mesh
    for mesh in scene["meshes"]:
        name = mesh["name"]
        file = mesh["file"]

        indexCount = mesh["indexCount"]
        wireCount = mesh["wireCount"]
        vertexCount = mesh["vertexCount"]

        texCoord2 = 0
        if "secondaryTexCoord" in mesh:
            texCoord2 = mesh["secondaryTexCoord"]
        vertexColor = 0
        if "vertexColor" in mesh:
            vertexColor = mesh["vertexColor"]

        indexTypeSize = mesh["indexTypeSize"]

        stride = 32
        if vertexColor > 0: stride = stride + 4
        if texCoord2 > 0: stride = stride + 8

        fdat = find(files["model/mset"], "filename", file)["data"]
        df = NoeBitStream(fdat)

        posList = []
        uvsList = []
        subMeshes = []

        for i, subMesh in enumerate(mesh["subMeshes"]):
            material = subMesh["material"]
            indexCount2 = subMesh["indexCount"]
            wireIndexCount2 = subMesh["wireIndexCount"]

            idxList = []

            for f in range(indexCount2):
                if indexTypeSize == 2:
                    idxList.append(df.readUShort())
                else:
                    idxList.append(df.readUInt())

            mesh = NoeMesh(idxList, posList)
            mesh.setName("%s_%d" % (name, i))
            mesh.setMaterial(material)
            subMeshes.append(mesh)

        df.seek(wireCount * indexTypeSize, 1)

        for v in range(vertexCount):
            posList.append(NoeVec3.fromBytes(df.readBytes(12)))
            uvsList.append(NoeVec3([df.readFloat(), -df.readFloat(), 0]))
            df.readBytes(stride - 20)

        for mesh in subMeshes:
            mesh.setPositions(posList)
            mesh.setUVs(uvsList)
            meshes.append(mesh)

    mdl = NoeModel(meshes)
    mdl.setModelMaterials(NoeModelMaterials(texList, matList))
    mdlList.append(mdl)

    return 1

def extract(bs):
    files = {}
    files["image/derp"] = []
    files["application/json"] = []
    files["image/jpeg"] = []
    files["image/png"] = []
    files["model/mset"] = []

    while not bs.checkEOF():
        name = bs.readString()
        ftype = bs.readString()
        c = bs.readUInt()
        d = bs.readUInt()
        e = bs.readUInt()
        bin = bs.readBytes(d)
        if c & 1:
            bin = decompress(bin, e)
        files[ftype].append({ "filename": name, "data": bin })
    return files

def decompress(a, b):
    c = bytearray(b)
    d = 0
    e = [0] * 4096
    f = [0] * 4096
    g = 256
    h = len(a)
    k = 0
    l = 1
    m = 0
    n = 1

    c[d] = a[0]
    d += 1

    r = 1
    while True:
        n = r + (r >> 1)
        if (n + 1) >= h:
            break
        m = a[n + 1]
        n = a[n]
        p = (m << 4 | n >> 4) if r & 1 else ((m & 15) << 8 | n)
        if p < g:
            if 256 > p:
                m = d
                n = 1
                c[d] = p
                d += 1
            else:
                m = d
                n = f[p]
                p = e[p]
                q = p + n
                while p < q:
                    c[d] = c[p]
                    d += 1
                    p += 1
        elif p == g:
            m = d
            n = l + 1
            p = k
            q = k + l
            while p < q:
                c[d] = c[p]
                d += 1
                p += 1
            c[d] = c[k]
            d += 1
        else:
            break

        e[g] = k
        f[g] = l + 1
        g += 1
        k = m
        l = n
        g = 256 if 4096 <= g else g
        r += 1

    return c if d == b else None

def find(lst, key, value):
    for i, dic in enumerate(lst):
        if dic[key] == value:
            return lst[i]
    return -1

def loadTex(files, fname):
    ftype = os.path.splitext(fname)[1]

    data = None
    if ftype == ".jpg":
        data = find(files["image/jpeg"], "filename", fname)["data"]
    if ftype == ".png":
        data = find(files["image/png"], "filename", fname)["data"]

    tex = rapi.loadTexByHandler(data, ftype)
    tex.name = fname

    return tex
