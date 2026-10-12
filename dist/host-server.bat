@echo off
rem Runs a public LP2 Online server: it appears in everyone's match search.
cd /d "%~dp0"
set /p NAME=Server name for the list: 
echo.
echo Forward port 27900 (TCP and UDP) on your router to this PC, and allow lp2-master
echo through Windows Firewall when asked. Keep this window open while people play.
echo.
lp2-master.exe 27900 --name "%NAME%"
pause
