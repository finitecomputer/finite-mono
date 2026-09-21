// Local disposable fixture: kubectl-ate currently has no egress-policy verb.
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/agent-substrate/substrate/internal/ateclient"
	"github.com/agent-substrate/substrate/pkg/proto/ateapipb"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/types/known/emptypb"
)

func main() {
	ctx := context.Background()
	c, err := ateclient.NewClient(ctx, os.Getenv("KUBECONFIG"), "kind-finite-hermes-spike", "", "", false)
	if err != nil {
		panic(err)
	}
	defer c.Close()
	for _, name := range os.Args[1:] {
		_, err = c.CreateActorEgressPolicy(ctx, &ateapipb.CreateActorEgressPolicyRequest{
			Actor:        &ateapipb.ObjectRef{Atespace: "finite-hermes-spike", Name: name},
			EgressPolicy: &ateapipb.EgressPolicy{Metadata: &ateapipb.ResourceMetadata{Atespace: "finite-hermes-spike", Name: "default"}, Rules: []*ateapipb.EgressRule{{All: &emptypb.Empty{}}}},
		})
		if err != nil && status.Code(err) != codes.AlreadyExists {
			panic(err)
		}
		fmt.Println(name, "local fixture egress policy ready")
	}
}
