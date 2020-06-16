MView File Viewer
=================

This project consists of 2 scripts. `extract_mview` to extract files inside the mview archive and `extract_model` to convert the `.dat` files to `.obj`.

Also includes a Noesis plugin. (see below)

**Example**

File downloaded from [ArtStation](https://www.artstation.com/artwork/3LBbA).

**Result**
![](http://i.imgur.com/EFu0Hg1.png)

Requirements
============

- Python 3.6.1 or later [[download](https://www.python.org/downloads/)]

Usage
=====

    // extract mview archive
    // python extract_mview.py <filename>
    $ python extract_mview.py test_data/test_file1.mview
    $ > thumbnail.jpg image/jpeg
    $ > sky.dat image/derp
    $ > mesh0.dat model/mset
    $ > mesh1.dat model/mset
    $ > mesh2.dat model/mset
    $ > mesh3.dat model/mset
    $ > mesh4.dat model/mset
    $ > mesh5.dat model/mset
    $ > mesh6.dat model/mset
    $ > mat0_c.jpg image/jpeg
    $ > mat0_r.jpg image/jpeg
    $ > mat0_n.jpg image/jpeg
    $ > mat0_a.jpg image/jpeg
    $ > mat0_g.jpg image/jpeg
    $ > mat0_s.jpg image/jpeg
    $ > mat1_c.jpg image/jpeg
    $ > mat1_n.jpg image/jpeg
    $ > mat2_c.jpg image/jpeg
    $ > mat3_c.jpg image/jpeg
    $ > mat4_c.jpg image/jpeg
    $ > mat4_a.jpg image/jpeg
    $ > mat5_c.jpg image/jpeg
    $ > mat5_a.jpg image/jpeg
    $ > mat5_s.jpg image/jpeg
    $ > mat6_c.jpg image/jpeg
    $ > mat6_a.jpg image/jpeg
    $ > scene.json.sig application/json
    $ > scene.json application/json
    $ > COMPLETED!!!

    // convert dat files to obj (wavefront)
    // python extract_model.py <folder_containing_scene.json>
    $ python extract_model.py test_data/test_file1
    $ > COMPLETED!!!

Viewer
======

You can download Noesis from [here](https://richwhitehouse.com/index.php?content=inc_projects.php&showproject=91). Copy and paste the plugin to `noesis/plugins/python/fmt_artstation_mview.py` then just open the `.mview` files with Noesis.

![](http://i.imgur.com/LgUFvEF.png)

Notes
=====

To download an `.mview` file:

1. Open url with 3D viewer in browser. (do not click play yet)
2. Open **Developer Tools** and go to **Network** tab.
3. Click the play button on the 3D viewer.
4. Type or search for `mview` on the **Developer Tools' Network** tab.
5. Right click on the file and select `open in new tab`. (will start download)

**[FIXED]** ~~[BUG] There is currently no support for huge files that uses uint32 indices. Pull requests are welcome.~~

Community
=========

- [Xentax Forum](http://forum.xentax.com) @majidemo, @shakotay2, @TaylorMouse
- [Marmoset Toolbag](https://www.marmoset.co/viewer)
- [ArtStation](https://www.artstation.com/artwork/3LBbA)
