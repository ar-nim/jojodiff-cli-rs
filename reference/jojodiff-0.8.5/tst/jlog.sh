#!/bin/bash

  #ruler   ---------1---------2---------3---------4---------5---------6---------7---------8
  #ruler   12345678901234567890123456789012345678901234567890123456789012345678901234567890
  #echo -e "LOG      HDR JDF-file             TIME            DATA /    OVH   /    EQL   /   ERR    OPTIONS"

for f
do
  g=${f%.*}
  printf "\n%-20.20s     SIZE     TIME            DATA /    OVH   /    EQL   /   ERR    OPTIONS\n" "$g"
  sta=""
  cat "$f" |
  while read l
  do
    if [[ -n "$sta" ]] #"$l" == "------"* ]]
    then
      if [[ -n "$sta" ]]
      then
        printf "%s %-20.20s %8s %-11.11s %8s / %8s / %8s / %8s %s\n" \
              "$sta" "${dif#*/}" "$sze" "$tme" "$dtabyt" "$ovhbyt" "$eqlbyt" "$misses" "$opt"
        sta=""
      fi
    elif [[ "$l" == */jdiff* ]]
    then
      opt=""
      dif=""
      s=""
      d=""
      c="${l##*jdiff}"
      while [[ -n "$c" ]]
      do
        c="${c# }"
        w="${c%% *}"
        c="${c#* }"
        if [[ "$w" == "$c" ]]; then c=""; fi
        #echo ":$w:$c:"
        if [[ "$w" == "-"* ]]
        then
          opt="$opt $w"
        elif [[ -z $w ]]; then :
        elif [[ -z "$s" ]]; then s="$w"
        elif [[ -z "$d" ]]; then d="$w"
        elif [[ -z "$out" ]]; then dif="$w"
        fi
      done
    elif [[ "$l" == Data*bytes* ]]
    then
      dtabyt="${l##*= }"
    elif [[ "$l" == Control*bytes* ]]
    then
      ovhbyt="${l##*= }"
    elif [[ "$l" == Overhead*bytes* ]]
    then
      ovhbyt="${l##*= }"
    elif [[ "$l" == Equal*bytes* ]]
    then
      eqlbyt="${l##*= }"
    elif [[ "$l" == Hashtable*misses* ]]
    then
      misses="${l##*= }"
    elif [[ "$l" == Search*errors* ]]
    then
      misses="${l##*= }"
    elif [[ "$l" == JDF* ]]
    then
      sta=JDF
      l="${l//$'\t'/ }"
      l="${l//  / }"
      l="${l//  / }"
      l="${l//  / }"
      l="${l//  / }"
      #echo ">$l"
      l="${l#* }" # remove first word: JDF
      l="${l#* }" # remove second word: filename
      l="${l%% *}" # keep third word: filesize
      sze="$l"
      #echo ">$sze"
    elif [[ "$l" == ERR* ]]
    then
      sta=ERR
    elif [[ "$l" == user* ]]
    then
      tme="${l##user?}"
    fi
  done
done
