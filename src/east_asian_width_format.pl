#!/usr/bin/perl -w

die unless open IN, "<EastAsianWidth.txt";
my @list = ();
while (<IN>) {
    s/\s*#.*$//;
    if (/;[WF]/) {
        s/;.*//;
        if (/^([a-fA-F0-9]+)$/) {
            push @list, "0x$1";
            push @list, "0x$1";
        } elsif (/^([a-fA-F0-9]+)\.\.([a-fA-F0-9]+)$/) {
            push @list, "0x$1";
            push @list, "0x$2";
        } else {
            die "Bad line: $_";
        }
    }
}

# Merge contiguous ranges
my @out = ();
my $c0 = shift @list;
my $c1 = shift @list;
while (@list) {
    my $n0 = shift @list;
    my $n1 = shift @list;

    if (hex($c1) + 1 == hex($n0)) {
        $c1 = $n1;
    } else {
        push @out, $c0;
        push @out, $c1;
        $c0 = $n0;
        $c1 = $n1;
    }
}
push @out, $c0;
push @out, $c1;

print "match cp {\n";
while (@out) {
    my $c0 = shift @out;
    my $c1 = shift @out;
    print "    $c0..=$c1 => true,\n";
}
print "    _ => false\n";
print "}\n";
