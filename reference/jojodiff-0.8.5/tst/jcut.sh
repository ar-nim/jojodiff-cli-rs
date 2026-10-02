#!/bin/bash
if [[ $# -lt 3 ]]
then
  echo "usage: ${0##*/} <file> <pos> <len>"
  echo "<pos> starts from 0"
  exit
fi
pos=$2
sze=$3

if ((sze<0))
then
  ((sze=sze*-1))
else
  ((pos=pos+sze-1))
fi

#head --bytes=$pos $1 | tail --bytes=$sze | pr -tv
od -j $pos -N $sze -Ad -txz "$1"
