#!/bin/bash
vrs=$1; shift
lst="- l-w32 n s -w32"
if [[ $# -gt 0 ]]
then
  lst="$*"
fi
for x in $lst
do 
  echo $x
done
#exit
for x in $lst
do 
  if [[ "$x" = '-' ]]
  then
    x=''
  fi
  if [[ -f ./jdiff$x.exe ]]
  then 
    if [[ $x = *w32 ]]
    then
       ./jtst ./jdiff$x.exe ./jptch$x.exe w32 all tst defs | tee jtst$vrs$x.log
    else
       ./jtst ./jdiff$x.exe ./jptch$x.exe cyg all tst defs | tee jtst$vrs$x.log
    fi
  fi
done
