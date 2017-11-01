# Windows Batch Script
I wrote this batch script to avoid typing in the same commands over and over. I wanted a simple, drag and drop solution.

## Setting up the batch script

There are 2 steps to getting this batch script ready for your environment:
 
1. Install Python 3.6.1 or later and make sure to check the "Add Python to your PATH" during the installation wizard. If you've already installed it and did not add Python to your PATH follow the instructions [here](https://superuser.com/questions/143119/how-to-add-python-to-the-windows-path) to set that up. The end goal is to have access to python from your command prompt by calling `py`
2. Edit the batch file and specify the path to the Python scripts `extract_mview.py` and `extract_model.py`

## Using the batch script
Once you've downloaded and edited the batch script to suit your environment there's only one step left.

1. Drag the `.mview` file onto the batch script and watch the magic happen, it will prompt you to press any key to continue between steps, if you want an even faster experience you can remove the calls to `pause` in the batch script
