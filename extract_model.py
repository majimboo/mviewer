#!/usr/bin/python
#@majidemo

import json, math, os, sys
from struct import *


def decode_normal(packed_x, packed_y):
    """Decode octahedral-encoded normal from two unsigned shorts.
    Matches Marmoset's matvert.glsl ic() function."""
    x_f = packed_x / 65535.0
    y_f = packed_y / 65535.0
    z_neg = y_f > (32767.1 / 65535.0)
    if z_neg:
        y_f -= 32768.0 / 65535.0
    nx = (2.0 * 65535.0 / 32767.0) * x_f - 1.0
    ny = (2.0 * 65535.0 / 32767.0) * y_f - 1.0
    nz_sq = max(0.0, 1.0 - nx * nx - ny * ny)
    nz = math.sqrt(nz_sq)
    if z_neg:
        nz = -nz
    return (nx, ny, nz)

def main(folder):
    f = open("%s/scene.json" % (folder))
    data = json.load(f)
    f.close()

    omtl = open("%s/master.mtl" % (folder), "w")
    for mat in data["materials"]:
        name = mat["name"]
        diffuse = mat["albedoTex"]
        # specular = mat["extrasTex"]

        # write to file
        omtl.write("newmtl {0}\n".format(name))
        omtl.write("map_Ka {0}\n".format(diffuse))
        omtl.write("map_Kd {0}\n".format(diffuse))
        # omtl.write("map_Ks {0}\n\n".format(specular))

    omtl.close()

    for mesh in data["meshes"]:
        name = mesh["name"]
        dat = mesh["file"]
        print("converting %s" % dat)
        # transform = mesh["transform"]
        wire_count = mesh["wireCount"]
        index_count = mesh["indexCount"]
        vertex_count = mesh["vertexCount"]

        tex_coord_2 = 0
        if "secondaryTexCoord" in mesh:
            tex_coord_2 = mesh["secondaryTexCoord"]

        vertex_color = 0
        if "vertexColor" in mesh:
            vertex_color = mesh["vertexColor"]

        index_type_size = mesh["indexTypeSize"]
        # consts
        stride = 32
        if vertex_color > 0: stride = stride + 4
        if tex_coord_2 > 0: stride = stride + 8

        # TODO: BUG LONG INDICES
        # if index_type_size == 4:
        #     raise Exception("ERROR! Currently can't process any large files with long (uint32) indices... To Be Updated!!!")

        # read stream
        df = open("%s/%s" % (folder, dat), "rb")
        # write stream
        output = open("{0}/{1}.obj".format(folder, dat), "w")
        output.write("mtllib master.mtl\n")

        # lists
        face_list = []
        vert_list = []
        uv_list = []
        normal_list = []
        materials_list = []

        for sub_mesh in mesh["subMeshes"]:
            faces = []
            material = sub_mesh["material"]
            index_count_2 = sub_mesh["indexCount"]
            wire_count_2 = sub_mesh["wireIndexCount"]

            face_count = int((index_count_2 * index_type_size) / 6)
            if index_type_size == 4:
                face_count = int((index_count_2 * index_type_size) / 12)

            # faces
            for f in range(face_count):
                if index_type_size == 2:
                    faces.append(unpack("<HHH", df.read(6)))
                else:
                    faces.append(unpack("<III", df.read(12)))

            # set submesh data
            face_list.append(faces)
            materials_list.append(material)

        # skip unknown wire count
        df.seek(wire_count * index_type_size, 1)

        # vertices
        for v in range(vertex_count):
            # position
            pos = unpack("<fff", df.read(12))
            # texcoord
            texpos = unpack("<ff", df.read(8))
            # secondary texcoord (if present)
            if tex_coord_2 > 0:
                df.read(8)
            # tangent + bitangent (skip)
            df.read(4)  # vTangent: 2 x unsigned short
            df.read(4)  # vBitangent: 2 x unsigned short
            # normal: 2 x unsigned short (octahedral encoded)
            packed_normal = unpack("<HH", df.read(4))
            normal = decode_normal(packed_normal[0], packed_normal[1])
            # vertex color (if present)
            if vertex_color > 0:
                df.read(4)

            vert_list.append(pos)
            uv_list.append(texpos)
            normal_list.append(normal)

        for vert in vert_list:
            output.write("v {0} {1} {2}\n".format(vert[0], vert[1], vert[2]))

        for uv in uv_list:
            output.write("vt {0} {1}\n".format(uv[0], uv[1]))

        for n in normal_list:
            output.write("vn {0:.6f} {1:.6f} {2:.6f}\n".format(n[0], n[1], n[2]))

        for x, faces in enumerate(face_list):
            output.write("\n")
            output.write("g {0}\n".format(name))
            output.write("usemtl {0}\n".format(materials_list[x]))

            for face in faces:
                output.write("f {0}/{0}/{0} {1}/{1}/{1} {2}/{2}/{2}\n".format(face[0]+1, face[1]+1, face[2]+1))

        df.close()
        output.close()

    print("COMPLETED!!!")

def mkDIR(dir):
    if not os.path.exists(dir):
        os.makedirs(dir)

if __name__ == "__main__":
    main(sys.argv[1])
