#!/bin/bash
if [[ $1 == "-f" ]]
then
  shift
  tl=1
  tail -fn +0 "$@"
else
  cat "$@"
fi | grep -e '^XDF' -e '^JDF' -e '^ERR' -e '^HDR' 
exit
  if [[ $tl == 1 ]]
then
  less -Mr +F
else
  cat
fi
