#!/bin/bash
#
# JSYNC - Jojo's sync v0.2 (Beta) July 2002
# Copyright (C) 2002 Joris Heirbaut (joheirba@tijd.com)
#
# This program is free software. Terms of the GNU General Public Licence apply.
# This program is distributed WITHOUT ANY WARRANTY, not even the implied
# warranty of MERCHANTABILITY of FITNESS FOR A PARTICULAR PURPOSE.
#
# Author                Version Date        Description
# --------------------  ------- -------     -------------------------------------
# Joris Heirbaut        v0.1b   05-05-2002  Use uuencode + diff + gzip
# Joris Heirbaut        v0.2b   24-06-2002  Use jdiff + gzip
# Joris Heirbaut        v0.2b   04-07-2002  Cleanup and bugfixes
# Joris Heirbaut        v0.2c   05-07-2002  Options -s, -b and -j
# Joris Heirbaut	v0.2c	08-07-2002  Option -l
# Joris Heirbaut        v0.2c	09-07-2002  Exit code 1 from jdiff if no changes
# Joris Heirbaut        v0.2c   11-07-2002  Bugfix: use dif if jdf fails
# Joris Heirbaut        v0.2c   16-07-2002  Cygwin: remove \r from input to ed
# Joris Heirbaut        v0.4a   ??-10-2002  Do not test existanca of a file in input mode
# Joris Heirbaut        v0.4c   06-11-2002  Additional TAR arguments
# Joris Heirbaut                07-11-2002  Test existence of source file before backing up
#
silent=0
best=0
lowstor=0
interact=""
verbose=""
while getopts sbjliv opt
do
  case $opt in
    "s") silent=1;;
    "b") best=1;;
    "j") best=0;;
    "l") lowstor=1;;
    "i") interact="-i";;
    "v") verbose="-v";;
  esac
done
shift $((OPTIND-1))
dir="$1"
adir="$2"
rdir="$3"
arc="$4"
src="$5"
shift 5
tararg="$*"
if [[ -z "$tararg" ]]; then tararg="."; fi
fil=${src##*/}

# test arguments
err=0
if [[ ($dir != "inp") && ($dir != "out") && ($dir != "upd")]]; then err=1
elif [[ ! ( -f "$src" ) && ! ( -d "$src" ) && ("$dir" == "out") ]]; then err=2
elif [[ ! -d "$adir" ]]; then err=3
elif [[ ! -d "$rdir" ]]; then err=4 
fi

if ((silent == 0 || err != 0))
then
  echo "JSYNC - Jojo's sync v0.2 (Beta) July 2002"
fi
if [[ $err -ne 0 ]]
then
  echo "Copyright (C) 2002 Joris Heirbaut (joheirba@tijd.com)"
  echo 
  echo "This program is free software. Terms of the GNU General Public Licence apply."
  echo "This program is distributed WITHOUT ANY WARRANTY, not even the implied"
  echo "warranty of MERCHANTABILITY of FITNESS FOR A PARTICULAR PURPOSE."
  echo
  echo "usage: jsync [options] [inp|out|upd] <archivedir> <remotedir> <name> <source>"
  echo "description:"
  echo "  JSYNC synchronizes files or directories between two or more computers by"
  echo "  passing jdiff patch files through a remote directory."
  echo
  echo "  JSYNC will always first copy the missing patch files from the remote directory"
  echo "  to the (local) archive directory and bring the archive directory up-to-date."
  echo
  echo "  In output mode, JSYNC will copy from the source to the remote directory."
  echo "  In input mode,  JSYNC will copy from the remote directory to the source."
  echo "  In update mode, JSYNC will input only if new patch files were found."
  echo
  echo "  In case <source> is a directory, tar is used to create one source file."
  echo
  echo "  By default, jsync will store the actual data, every jdiff patch file and the"
  echo "  original. The archive directory then stores each intermediary state of the"
  echo "  source data (like an incremental backup), but will need 2 or more times the"
  echo "  size of the source data in storage."
  echo "  During operation, this will even go up to 4 to 5 times the source data." 
  echo
  echo "  When using the -l (low storage) option, jdiff will only store the last state"
  echo "  of the archive, reducing storage to \"only\" 1 times the source size and to"
  echo "  3 times the source size during operation."
  echo
  echo "arguments:"
  echo "  <archivedir>  directory in which archive and temporary files are created "
  echo "  <remotedir>   directory in which archive files are copied"
  echo "  <name>        name of the archive"
  echo "  <source>      the source file or directory"
  echo 
  echo "options:"
  echo "  -b	best:        take best of uuencode+diff vs. jdiff."
  echo "  -s    silent:      less screen output."
  echo "  -v    verbose:     more screen output."
  echo "  -l    low storage: remove data and jdiff files after they've been used."
  echo "  -i    interactive: ask before overwriting patch file."
  echo
fi
case $err in
  ("1") echo "Invalid mode $dir specified!";;
  ("2") echo "Source file/directory $src not found!";;
  ("3") echo "Archive directory $adir not found";;
  ("4") echo "Remote directory $rdir not found";;
esac
if [[ $err -ne 0 ]]; then exit 2 ; fi

date >>$adir/$arc.log
echo "jsync $*" >>$adir/$arc.log

{
cd "$adir"

# determine "nature": tar or fil
nat="fil"
if [[ -d "$src" ]]
then
  nat="tar"
fi

# get current level
uue=""
uue=$(ls "$arc".*.uue.gz 2>/dev/null | tail -1)
uue=${uue#*.}
uue=${uue%%.*}
uue=${uue#0}
uue=${uue#0}
uue=${uue#0}
if [[ -z "$uue" ]]
then
  uue=-1
fi
pak=$(ls "$arc".*.$nat.gz 2>/dev/null | tail -1)
pak=${pak#*.}
pak=${pak%%.*}
pak=${pak#0}
pak=${pak#0}
pak=${pak#0}
if [[ -z "$pak" ]]
then
  pak=-1
fi

cur=$uue
if [[ $pak -ge $cur ]]; then cur=$pak; fi
if ((silent==0))
then
  echo Current level = $cur
fi

# copy first data file from remote directory
if [[($cur -eq -1) && (-f "$rdir"/"$arc".0000.$nat.gz) ]]
then
  if ((silent==0)); then echo "Copy $arc.0000.$nat.gz from $rdir"; fi
  cp "$rdir"/"$arc".0000.$nat.gz .
  cur=0
fi

# copy patch files from remote directory
((lst=cur+1))
while [[ (-f "$rdir"/"$arc".$(printf %04d $lst).jdf.gz) || (-f "$rdir"/"$arc".$(printf %04d $lst).dif.gz) ]]
do
  new="$arc".$(printf %04d $lst)
  if [[ -f "$rdir"/"$new".jdf.gz ]]
  then
    new="$new".jdf.gz
  else
    new="$new".dif.gz
  fi

  if [[ (! -f "$new") ]]
  then
    if ((silent==0)); then echo "Copy $new from $rdir."; fi
    cp "$rdir"/"$new" .
  elif [[ "$rdir"/"$new" -nt "$new" ]]
  then
    if ((silent==0)); then echo "Copy $new from $rdir (newer file!)."; fi
    cp $interact "$rdir"/"$new" .
  fi

  ((lst=lst+1))
done

# get last increment
while [[ (-f "$arc".$(printf %04d $lst).jdf.gz) || (-f "$arc".$(printf %04d $lst).dif.gz) ]]
do
  ((lst=lst+1))
done
((lst=lst-1))

# check if something needs to be done
if [[ ( $dir == "upd" ) && ( $cur -ge $lst ) ]]
then
  if ((silent==0)); then echo "Input is up-to-date."; echo; fi
  exit 1
fi

# bring current level to last increment
for ((cur=$cur+1;lst>=cur;cur=$cur+1))
do
  ((prv=cur-1))
  new=$arc.$(printf %04d $cur)
  old=$arc.$(printf %04d $prv)

  # select diff and pack methods
  dif="dif"
  if [[ -f $new.jdf.gz ]]  ; then dif="jdf"; fi

  # uncompress
  if [[ -f $old.$nat.gz ]]; then gunzip $old.$nat.gz; fi
  if [[ -f $old.uue.gz ]];  then gunzip $old.uue.gz;  fi

  # convert package from uue to tar/fil or tar/fil to uue
  if [[ $dif == "jdf" && ! (-f $old.$nat) ]]
  then
    if [[ $verbose == "-v" ]]; then echo "Convert $old.uue to $old.$nat"; fi
    uudecode -o $old.$nat $old.uue
  elif [[ $dif == "dif" && ! (-f $old.uue) ]]
  then
    if [[ $verbose == "-v" ]]; then echo "Convert $old.$nat.gz to $old.uue.gz"; fi
    uuencode $old.$nat $fil >$old.uue
  fi

  # construct
  if [[ $dif = "dif" ]]
  then
    if ((silent==0)); then echo "Increment $cur: PATCH $old.uue with $new.dif.gz"; fi
    zcat $new.dif.gz | tr -d '\r' | ed -s $old.uue 
  else
    if ((silent==0)); then echo "Increment $cur: JPATCH $old.$nat with $new.jdf.gz"; fi
    zcat $new.jdf.gz | jpatch $old.$nat - $new.$nat
  fi
  
  # cleanup
  if ((lowstor==1))
  then
    rm $new.$dif.gz
  fi
  if ((prv==0 && lowstor==0))
  then
    if [[ -f $old.$nat ]]
    then
      gzip $old.$nat
      rm -f $old.uue
    else
      gzip $old.uue
    fi
  else
    rm -f $old.uue
    rm -f $old.$nat
  fi
done

# compare with source file
if [[ $dir == "out" ]]
then
  ((prv=cur-1))
  new=$arc.$(printf %04d $cur)
  old=$arc.$(printf %04d $prv)

  if ((cur==0))
  then
    if ((silent==0)); then echo "Increment $cur"; fi
  else
    if ((silent==0)); then echo "Increment $cur: DIFF $old $new"; fi
  fi

  # copy new data
  if [[ $verbose == "-v" ]]; then echo "  Reading $src" ; fi
  if [[ -d "$src" ]]
  then
    (cd $src ; tar $verbose -c $tararg >$adir/$new.tar )
  else
    cp "$src" $new.fil
  fi

  if ((cur==0))
  then
    # Start new archive
    if [[ $verbose == "-v" ]]; then echo "  Copy $new.$nat.gz to $rdir"; fi
    gzip $new.$nat
    cp -p $new.$nat.gz $rdir
  else
    # Uncompress and/or convert last archived data
    if [[ ! -f $old.$nat && -f $old.$nat.gz ]]
    then
      if [[ $verbose == "-v" ]]; then echo "  Uncompress $old.$nat"; fi
      zcat $old.$nat.gz >$old.$nat
    fi
    if [[ ! -f $old.uue && -f $old.uue.gz ]]
    then
      if [[ $verbose == "-v" ]]; then echo "  Uncompress $old.uue"; fi
      zcat $old.uue.gz >$old.uue
    fi
    if [[ ! -f $old.$nat ]]
    then
      if [[ $verbose == "-v" ]]; then echo "  Convert $old.uue to $old.$nat"; fi
      uudecode -o $old.$nat $old.uue
    fi

    # check for difference
    if [[ $verbose == "-v" ]]; then echo "  JDIFF $old.$nat $new.$nat $new.jdf"; fi
    jdiff $old.$nat $new.$nat $new.jdf

    # check output
    status=$?
    if [[ $status -eq 1 ]]
    then
      status=1
    elif [[ $status -eq 0 ]]
    then
      if $(jpatch $old.$nat $new.jdf | cmp -s $new.$nat)
      then
        status=0
        if [[ $verbose == "-v" ]]; then echo "  JDIFF OK"; fi
      else
        status=2
        if [[ $verbose == "-v" ]]; then echo "  JDIFF Bad patchfile!"; fi
        mv $new.jdf $new.bad
      fi
    else
      if ((silent==0)); then echo "  JDIFF Error $status"; fi
      status=2
    fi

    # try uuencode + diff
    if ((status==2 || (best==1 && status!=1)))
    then
      # convert new data to uue
      if [[ $verbose == "-v" ]]; then echo "  Convert $new.$nat to $new.uue"; fi
      uuencode $new.$nat $fil >$new.uue

      # convert old data to uue
      if [[ (! -f $old.uue) ]]
      then
        if [[ $verbose == "-v" ]]; then echo "  Convert $old.$nat to $old.uue"; fi
        uuencode $old.$nat $fil >$old.uue
      fi

      # check for difference
      if [[ $verbose == "-v" ]]; then echo "  DIFF $old.uue $new.uue $new.dif"; fi
      diff -e $old.uue $new.uue >$new.dif

      # check output
      if [[ ! -s $new.dif ]]
      then 
        rm -f $new.jdf
        rm -f $new.dif
        status=1
      else
        echo "wq $new.uue" >>$new.dif
      fi
    fi 

    # different?
    if ((status==1))
    then
      if ((silent==0)); then echo "  No differences for $arc"; fi
      rm -f $new.*
      ((cur=cur-1))
    else
      if [[ -f $new.dif ]]; then gzip $new.dif; fi
      if [[ -f $new.jdf ]]; then gzip $new.jdf; fi

      # get best of both files
      sze1=0
      sze2=0
      rsn=""
      if ((best==1))
      then
        sze1=$(ls --block-size=1 -s $new.dif.gz)
        sze2=$sze1
        if [[ -f $new.jdf.gz ]]
        then
          sze2=$(ls --block-size=1 -s $new.jdf.gz)
        fi
        sze1=${sze1%$new*} # drop text, keep number of bytes
        sze2=${sze2%$new*} # drop text, keep number of bytes
        rsn="($sze1 against $sze2)"
      fi
      if [[($sze1 -lt $sze2) || ($status -eq 2)]]
      then
        if ((silent==0)); then echo "  Copy $new.dif.gz to $rdir  $rsn"; fi
        cp -p $new.dif.gz $rdir 
        rm -f $new.jdf.gz
      else
        if ((silent==0)); then echo "  Copy $new.jdf.gz to $rdir  $rsn"; fi
        cp -p $new.jdf.gz $rdir 
        rm -f $new.dif.gz
      fi

      # cleanup old files
      if ((prv>0))
      then
        rm -f $old.$nat.gz
        rm -f $old.uue.gz
      fi
      rm -f $old.$nat
      rm -f $old.uue
    fi
  fi
else
  ((cur=cur-1))
fi
new=$arc.$(printf %04d $cur)

# compress last archive files
if [[ -f $new.$nat && ! (-f $new.$nat.gz)]]
then 
  if [[ $verbose == "-v" ]]; then echo "  Compress $new.$nat"; fi
  gzip $new.$nat
elif [[ -f $new.uue  && ! (-f $new.uue.gz)]]
then 
  if [[ $verbose == "-v" ]]; then echo "  Compress $new.uue"; fi
  gzip $new.uue
fi
if [[ -f $new.$nat.gz && -f $new.uue.gz ]]
then
  rm -f $new.uue.gz
fi

# cleanup uncompressed files
rm -f $verbose $arc.*.tar
rm -f $verbose $arc.*.fil
rm -f $verbose $arc.*.uue
rm -f $verbose $arc.*.dif
rm -f $verbose $arc.*.jdf
rm -f $verbose ${src##*/}

# cleanup old data and patch files
if ((lowstor==1))
then
  rm -f $verbose $arc.*.jdf.gz
  rm -f $verbose $arc.*.dif.gz
  if ((cur>0))
  then
    rm -f $verbose $arc.0000.fil.gz
    rm -f $verbose $arc.0000.tar.gz
  fi
fi

# replace sourcefile
if [[ $dir == "inp" || $dir == "upd" ]]
then
  if [[ -e "$src" ]]
  then
    if ((silent==0)); then echo Backup "$src"; fi
    if [[ -d "$src" ]]
    then
      (cd $src ; tar $verbose -c $tararg | gzip >$adir/$arc.bak.gz )
    else
      gzip -c "$src" >$arc.bak.gz
    fi
  fi
  
  if ((silent==0)); then echo Writing "$src"; fi
  if [[ -f $new.tar.gz ]]
  then
    (cd $src; zcat $adir/$new.tar.gz | tar -x $verbose)
  elif [[ -f $new.fil.gz ]]
  then
    zcat $new.fil.gz >"$src"
  elif [[ -f $new.uue.gz ]]
  then
    zcat $new.uue.gz | uudecode -o "$src"
  else
    echo "Archive file missing!"
  fi
fi
echo
} 2>&1 | tee -a $adir/$arc.log
exit 0
