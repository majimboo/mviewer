#!/usr/bin/python
#@majidemo

import json, os, sys
from struct import *

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
            # stride
            df.read(stride - 20)

            vert_list.append(pos)
            uv_list.append(texpos)

        for vert in vert_list:
            output.write("v {0} {1} {2}\n".format(vert[0], vert[1], vert[2]))

        for uv in uv_list:
            output.write("vt {0} {1}\n".format(uv[0], uv[1]))

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

main(sys.argv[1])