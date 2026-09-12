@echo off
title T.U.S.H.E.R Node 2 (Secondary PC)
echo ========================================================
echo   T.U.S.H.E.R - Decentralized Mesh Node (Secondary PC)
echo ========================================================
echo.
echo Starting T.U.S.H.E.R background daemon on port 42426...
echo Web Dashboard will be available at: http://127.0.0.1:42951
echo.
tusher-desktop.exe --name "TusherPC-2" --port 42426 --discovery-port 42427 --web-port 42951 --data-dir ".\data" --open
pause