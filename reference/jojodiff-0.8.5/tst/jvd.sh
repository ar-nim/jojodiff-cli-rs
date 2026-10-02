#!/bin/bash
if [[ $# -lt 5 ]]
then
  echo "usage: ${0##*/} <file1> <file2> <pos1> <pos2> <len> [<off>]"
  echo "<pos> starts from 0"
  exit
fi
pos1=$3
pos2=$4
sze=$5
if [[ $# -gt 5 ]]
then
  off=$6
  ((pos1=pos1+off))
  ((pos2=pos2+off))
  ((len=len+off))
fi


if ((sze<0))
then
  ((sze=sze*-1))
else
  ((pos=pos+sze-1))
fi

#head --bytes=$pos $1 | tail --bytes=$sze | pr -tv
vim -d <( od -j $pos1 -N $sze -Ad -tx1z "$1" ) <( od -j $pos2 -N $sze -Ad -tx1z "$2" ) 
