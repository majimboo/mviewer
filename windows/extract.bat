@echo off
set filepath=%1
echo %filepath%
py F:\Downloads\mviewer-master\mviewer-master\extract_mview.py %filepath%
set dir=%filepath:~0,-6%
echo %dir%
pause
py F:\Downloads\mviewer-master\mviewer-master\extract_model.py %dir%
pause null