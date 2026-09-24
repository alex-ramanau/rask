#!/usr/bin/perl

# The Perl oracle for the regex spike.
#
#   perl-matches.pl patterns.jsonl haystack.bin > perl-results.jsonl
#
# patterns.jsonl: {"id":N,"pattern":"...","mods":"..."} per line.
# haystack.bin:   raw bytes, split into lines on "\n" (terminator kept).
#
# For each pattern, prints the compile error, or every match as
# [line, start, end] byte offsets: "first" is the first match in each line,
# "all" is what while (m//g) iterates over.

use strict;
use warnings;
no re 'eval';

use JSON::PP ();

my ( $patterns_file, $haystack_file ) = @ARGV;
my $json = JSON::PP->new->canonical->ascii;

open( my $hfh, '<:raw', $haystack_file ) or die "$haystack_file: $!";
my @lines = <$hfh>;
close $hfh;

open( my $pfh, '<', $patterns_file ) or die "$patterns_file: $!";
while ( my $rec = <$pfh> ) {
    my $p = $json->decode($rec);
    # JSON gives character strings. Patterns from ack are ASCII or bytes; keep them as bytes.
    my $pat = $p->{pattern};
    utf8::downgrade( $pat, 1 );
    my $mods = $p->{mods} // '';

    local $SIG{__WARN__} = sub { CORE::die @_ };
    my $re = eval { $mods ne '' ? qr/(?$mods)$pat/ : qr/$pat/ };
    if ( !$re ) {
        my $err = $@;
        chomp $err;
        print $json->encode( { id => $p->{id}, error => $err } ), "\n";
        next;
    }

    my ( @first, @all );
    for my $n ( 0 .. $#lines ) {
        my $line = $lines[$n];
        my $seen_first = 0;
        while ( $line =~ /$re/g ) {
            my $m = [ $n, $-[0], $+[0] ];
            push @first, $m unless $seen_first++;
            push @all, $m;
        }
    }
    print $json->encode( { id => $p->{id}, first => \@first, all => \@all } ), "\n";
}
